use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use vita_save::{
    backup::Store,
    cloud::{WebDav, parse_listing},
    config::Config,
    job::Control,
    saves::Game,
};
struct Server {
    url: String,
    files: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Server {
    fn start() -> Self {
        Self::with_statuses(201, 204)
    }
    fn with_statuses(put_status: u16, delete_status: u16) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let files = Arc::new(Mutex::new(BTreeMap::<String, Vec<u8>>::new()));
        let state = files.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let worker = std::thread::spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                let (stream, _) = match listener.accept() {
                    Ok(v) => v,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(e) => panic!("{e}"),
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let mut parts = line.split_whitespace();
                let method = parts.next().unwrap_or("").to_owned();
                let path = parts.next().unwrap_or("").to_owned();
                let mut headers = BTreeMap::new();
                loop {
                    line.clear();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                    let (k, v) = line.split_once(':').unwrap();
                    headers.insert(k.to_ascii_lowercase(), v.trim().to_owned());
                }
                let len = headers
                    .get("content-length")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0usize);
                assert!(len < 16 * 1024 * 1024);
                let mut body = vec![0; len];
                reader.read_exact(&mut body).unwrap();
                let mut files = state.lock().unwrap();
                let (code, body) = match method.as_str() {
                    "MKCOL" => (201, Vec::new()),
                    "PUT" => {
                        assert_eq!(headers["if-none-match"], "*");
                        if files.contains_key(&path) {
                            (412, Vec::new())
                        } else {
                            if put_status == 201 {
                                files.insert(path.clone(), body);
                            }
                            (put_status, Vec::new())
                        }
                    }
                    // Reproduce a service that accepts PUT but cannot MOVE the object.
                    "MOVE" => (404, Vec::new()),
                    "GET" => files
                        .get(&path)
                        .map(|data| (200, data.clone()))
                        .unwrap_or((404, Vec::new())),
                    "DELETE" => {
                        if delete_status == 204 {
                            files.remove(&path);
                        }
                        (delete_status, Vec::new())
                    }
                    "PROPFIND" => {
                        let rows=files.keys().filter(|p|p.starts_with(&path)).map(|p|format!("<D:response><D:href>{p}</D:href><D:propstat><D:prop/><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>")).collect::<String>();
                        (207,format!("<?xml version=\"1.0\"?><D:multistatus xmlns:D=\"DAV:\">{rows}</D:multistatus>").into_bytes())
                    }
                    _ => (405, Vec::new()),
                };
                drop(files);
                let stream = reader.get_mut();
                write!(
                    stream,
                    "HTTP/1.1 {code} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(&body).unwrap();
            }
        });
        Self {
            url: format!("http://{address}/dav/"),
            files,
            stop,
            worker: Some(worker),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}
#[test]
fn upload_works_without_move_and_preserves_existing_versions() {
    let server = Server::start();
    let config = Config {
        webdav_url: server.url.clone(),
        webdav_user: "tester".into(),
        webdav_password: "test-only".into(),
        ..Default::default()
    };
    let cloud = WebDav::new(&config).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("save");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("data.bin"), vec![42; 130_000]).unwrap();
    let game = Game {
        title_id: "PCSG00001".into(),
        save_id: "SHARED001".into(),
        name: "Game".into(),
        path: source.clone(),
        icon: None,
    };
    let store = Store::new(tmp.path().join("local"));
    let c = Control::default();
    let m = store.create(&game, &source, false, &c).unwrap();
    cloud.test(&c).unwrap();
    assert!(
        server.files.lock().unwrap().is_empty(),
        "connection probe must be cleaned up"
    );
    cloud.upload(&store, &game, &m.id, &c).unwrap();
    assert_eq!(cloud.list(&game, &c).unwrap()[0].id, m.id);
    let second = Store::new(tmp.path().join("download"));
    cloud.download(&second, &game, &m.id, &c).unwrap();
    assert_eq!(second.verify(&m.id, &c).unwrap(), m);
    assert!(cloud.upload(&store, &game, &m.id, &c).is_err());
    let files = server.files.lock().unwrap();
    assert_eq!(files.len(), 1);
    assert!(files.keys().all(|p| p.ends_with(".vsave")));
}

#[test]
fn connection_test_rejects_read_only_and_reports_cleanup_errors() {
    for (put, delete, operation) in [(403, 204, "PUT"), (201, 403, "DELETE")] {
        let server = Server::with_statuses(put, delete);
        let cloud = WebDav::new(&Config {
            webdav_url: server.url.clone(),
            ..Default::default()
        })
        .unwrap();
        let error = format!("{:#}", cloud.test(&Control::default()).unwrap_err());
        assert!(error.contains(operation), "{error}");
        assert!(error.contains("403"), "{error}");
    }
}

#[test]
fn connection_test_finishes_on_the_settings_page() {
    use vita_save::{
        app::App,
        platform::Environment,
        ui::{Action, Page, Tab},
    };
    let server = Server::start();
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("savedata")).unwrap();
    let mut app = App::new(Environment::host(tmp.path())).unwrap();
    let ready = |app: &mut App| {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while app.model.busy {
            assert!(std::time::Instant::now() < deadline);
            app.poll().unwrap();
            std::thread::yield_now();
        }
    };
    ready(&mut app);
    app.model.config.webdav_url = server.url.clone();
    app.dispatch(Action::Settings).unwrap();
    app.dispatch(Action::SettingsTab(Tab::WebDav)).unwrap();
    app.dispatch(Action::TestConnection).unwrap();
    assert_eq!(app.view.page(), &Page::Settings(Tab::WebDav));
    assert!(!app.view.shortcut_allowed());
    ready(&mut app);
    assert_eq!(app.view.page(), &Page::Settings(Tab::WebDav));
    assert_eq!(app.model.message, app.view.text(&app.model, "connected"));
    assert!(server.files.lock().unwrap().is_empty());
}
#[test]
fn listing_rejects_external_paths_and_dtd() {
    let base = url::Url::parse("https://example.com/dav/vita-save/PCSG00001/SAVE001/").unwrap();
    let xml=br#"<multistatus xmlns="DAV:"><response><href>abc-123.vsave</href></response><response><href>https://evil.example/abc.vsave</href></response><response><href>../abc.vsave</href></response><response><href>sub/abc.vsave</href></response></multistatus>"#;
    let entries = parse_listing(&base, xml).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, "abc-123");
    assert!(parse_listing(&base, br#"<!DOCTYPE root><root/>"#).is_err());
}
#[test]
fn redirects_are_rejected_without_forwarding_credentials() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let worker = std::thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        let mut data = [0u8; 4096];
        let _ = s.read(&mut data).unwrap();
        s.write_all(b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/credentials\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
    });
    let cloud = WebDav::new(&Config {
        webdav_url: format!("http://{address}/"),
        ..Default::default()
    })
    .unwrap();
    assert!(cloud.test(&Control::default()).is_err());
    worker.join().unwrap();
}

#[test]
fn listing_decodes_entities_and_cdata_in_server_paths() {
    let base = url::Url::parse("https://example.com/dav/a&b/").unwrap();
    for href in [
        "/dav/a&amp;b/abc-123.vsave",
        "/dav/a&#38;b/abc-123.vsave",
        "<![CDATA[/dav/a&b/abc-123.vsave]]>",
    ] {
        let xml = format!("<multistatus><response><href>{href}</href></response></multistatus>");
        let entries = parse_listing(&base, xml.as_bytes()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "abc-123");
    }
    assert!(parse_listing(&base, b"<response><href>&unknown;</href></response>").is_err());
}
