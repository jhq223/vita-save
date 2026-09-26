pub mod archive;
mod transport;
use crate::{
    backup::{self, Manifest, Store},
    config::Config,
    job::Control,
    saves::Game,
};
use anyhow::{Context, Result, ensure};
use base64::Engine;
use quick_xml::{Reader, events::Event};
use std::{fs::File, io::Read, time::Duration};
use ureq::{
    Agent, Body,
    http::{Request, Response},
};
use url::Url;

#[derive(Clone, Debug)]
pub struct Remote {
    pub id: String,
}
pub struct WebDav {
    agent: Agent,
    base: Url,
    authorization: String,
}
impl WebDav {
    pub fn new(config: &Config) -> Result<Self> {
        let mut base = Url::parse(&config.webdav_url).context("Invalid WebDAV URL")?;
        ensure!(
            ["https", "http"].contains(&base.scheme()) && base.host_str().is_some(),
            "WebDAV needs an HTTP(S) URL"
        );
        ensure!(
            base.username().is_empty()
                && base.password().is_none()
                && base.query().is_none()
                && base.fragment().is_none(),
            "Put credentials in their own fields; URL must not contain query or fragment"
        );
        ensure!(
            !config.webdav_user.contains(':'),
            "WebDAV username cannot contain ':'"
        );
        if !base.path().ends_with('/') {
            base.set_path(&format!("{}/", base.path()));
        }
        let agent = Agent::config_builder()
            .ip_family(ureq::config::IpFamily::Ipv4Only)
            .max_redirects(0)
            .allow_non_standard_methods(true)
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_resolve(Some(Duration::from_secs(10)))
            .timeout_recv_response(Some(Duration::from_secs(30)))
            .timeout_send_request(Some(Duration::from_secs(30)))
            .timeout_global(None)
            .proxy(None)
            .build();
        let agent = transport::agent(agent);
        let authorization = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD
                .encode(format!("{}:{}", config.webdav_user, config.webdav_password))
        );
        Ok(Self {
            agent,
            base,
            authorization,
        })
    }
    fn url(&self, components: &[&str]) -> Result<Url> {
        let mut url = self.base.clone();
        let mut path = url
            .path_segments_mut()
            .map_err(|_| anyhow::anyhow!("Invalid base URL"))?;
        path.pop_if_empty();
        for component in components {
            ensure!(
                backup::valid_component(component),
                "Invalid remote path segment"
            );
            path.push(component);
        }
        drop(path);
        Ok(url)
    }
    fn request(&self, method: &str, url: &Url) -> ureq::http::request::Builder {
        Request::builder()
            .method(method)
            .uri(url.as_str())
            .header("Authorization", &self.authorization)
    }
    fn status(response: &Response<Body>, expected: &[u16]) -> Result<()> {
        let code = response.status().as_u16();
        ensure!(expected.contains(&code), "WebDAV HTTP {code}");
        Ok(())
    }
    pub fn test(&self, control: &Control) -> Result<()> {
        control.check()?;
        control.set("connect", "", 0, 0);
        let response = self.agent.run(
            self.request("PROPFIND", &self.base)
                .header("Depth", "0")
                .header("Content-Type", "application/xml; charset=utf-8")
                .body(PROPFIND)?,
        )?;
        Self::status(&response, &[207]).context("PROPFIND: read WebDAV directory")?;
        // Exercise the same write/read/delete operations used for backup files.
        let name = format!("vita-save-check-{}.tmp", backup::new_id()?);
        let probe = self.url(&[&name])?;
        let payload = b"Vita Save WebDAV connection check";
        control.check()?;
        let result = (|| {
            let response = self.agent.run(
                self.request("PUT", &probe)
                    .header("If-None-Match", "*")
                    .header("Content-Type", "application/octet-stream")
                    .body(payload.as_slice())?,
            )?;
            Self::status(&response, &[200, 201, 204]).context("PUT: write connection check")?;
            control.check()?;
            let mut response = self.agent.run(self.request("GET", &probe).body(())?)?;
            Self::status(&response, &[200]).context("GET: read connection check")?;
            let mut data = Vec::new();
            response
                .body_mut()
                .as_reader()
                .take(payload.len() as u64 + 1)
                .read_to_end(&mut data)?;
            ensure!(
                data == payload,
                "WebDAV connection check read-back mismatch"
            );
            Ok(())
        })();
        let cleanup = self
            .agent
            .run(self.request("DELETE", &probe).body(())?)
            .map_err(anyhow::Error::from)
            .and_then(|response| Self::status(&response, &[200, 204, 404]));
        match (result, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), Ok(())) => Err(error),
            (result, Err(cleanup)) => anyhow::bail!(
                "{}DELETE connection check {} failed: {cleanup:#}",
                result
                    .err()
                    .map_or(String::new(), |error| format!("{error:#}; ")),
                name
            ),
        }
    }
    fn collection(&self, game: &Game, create: bool, control: &Control) -> Result<Url> {
        let parts = ["vita-save", &game.title_id, &game.save_id];
        if create {
            for i in 1..=parts.len() {
                control.check()?;
                let response = self
                    .agent
                    .run(self.request("MKCOL", &self.url(&parts[..i])?).body(())?)?;
                Self::status(&response, &[201, 405]).context("MKCOL: create backup directory")?;
            }
        }
        self.url(&parts)
    }
    pub fn list(&self, game: &Game, control: &Control) -> Result<Vec<Remote>> {
        control.set("cloud_list", "", 0, 0);
        control.check()?;
        let mut url = self.collection(game, false, control)?;
        url.set_path(&format!("{}/", url.path()));
        let mut response = self.agent.run(
            self.request("PROPFIND", &url)
                .header("Depth", "1")
                .header("Content-Type", "application/xml; charset=utf-8")
                .body(PROPFIND)?,
        )?;
        if response.status().as_u16() == 404 {
            return Ok(Vec::new());
        }
        Self::status(&response, &[207]).context("PROPFIND: list cloud backups")?;
        let mut data = Vec::new();
        response
            .body_mut()
            .as_reader()
            .take(2 * 1024 * 1024 + 1)
            .read_to_end(&mut data)?;
        ensure!(
            data.len() <= 2 * 1024 * 1024,
            "WebDAV listing exceeds 2 MiB"
        );
        parse_listing(&url, &data)
    }
    pub fn upload(&self, store: &Store, game: &Game, id: &str, control: &Control) -> Result<()> {
        let manifest = store.read(id)?;
        ensure!(
            manifest.title_id == game.title_id && manifest.save_id == game.save_id,
            "Wrong snapshot for upload"
        );
        let stage = store.stage(id)?;
        let path = stage.path.join("upload.vsave");
        let mut file = File::create(&path)?;
        archive::export(store, id, &mut file, control)?;
        file.sync_all()?;
        drop(file);
        self.collection(game, true, control)?;
        ensure!(
            !self
                .list(game, control)?
                .iter()
                .any(|remote| remote.id == id),
            "This backup already exists in cloud storage"
        );
        let final_name = format!("{id}.vsave");
        let destination = self.url(&["vita-save", &game.title_id, &game.save_id, &final_name])?;
        control.check()?;
        let file = File::open(path)?;
        let total = file.metadata()?.len();
        let body = ureq::SendBody::from_owned_reader(ProgressReader {
            inner: file,
            control: control.clone(),
            done: 0,
            total,
        });
        let response = self.agent.run(
            self.request("PUT", &destination)
                .header("Content-Type", "application/octet-stream")
                .header("Content-Length", total)
                .header("If-None-Match", "*")
                .body(body)?,
        )?;
        Self::status(&response, &[200, 201, 204]).context("PUT: upload backup")?;
        // Do not depend on server-side rename support. Conditional PUT keeps
        // an existing version intact; downloads verify the complete archive.
        // An ambiguous transport failure must not trigger DELETE of a name
        // another device may have just created.
        Ok(())
    }
    pub fn download(
        &self,
        store: &Store,
        game: &Game,
        id: &str,
        control: &Control,
    ) -> Result<Manifest> {
        control.check()?;
        let name = format!("{id}.vsave");
        let url = self.url(&["vita-save", &game.title_id, &game.save_id, &name])?;
        let mut response = self.agent.run(self.request("GET", &url).body(())?)?;
        Self::status(&response, &[200]).context("GET: download backup")?;
        archive::import(
            store,
            id,
            &game.title_id,
            &game.save_id,
            response.body_mut().as_reader(),
            control,
        )
    }
}
const PROPFIND: &str = r#"<?xml version="1.0"?><d:propfind xmlns:d="DAV:"><d:prop><d:resourcetype/></d:prop></d:propfind>"#;

pub fn parse_listing(base: &Url, bytes: &[u8]) -> Result<Vec<Remote>> {
    let mut xml = Reader::from_reader(bytes);
    xml.config_mut().trim_text(true);
    let mut path = Vec::new();
    let mut href = String::new();
    let mut result = std::collections::BTreeSet::new();
    loop {
        match xml.read_event()? {
            Event::Start(e) => {
                path.push(e.local_name().as_ref().to_vec());
                ensure!(path.len() <= 32, "WebDAV XML is too deeply nested");
                if path.last().map(Vec::as_slice) == Some(b"href") {
                    href.clear();
                }
            }
            Event::Text(e) if path.last().map(Vec::as_slice) == Some(b"href") => {
                href.push_str(&e.decode()?);
            }
            Event::CData(e) if path.last().map(Vec::as_slice) == Some(b"href") => {
                href.push_str(&e.decode()?);
            }
            Event::GeneralRef(e) if path.last().map(Vec::as_slice) == Some(b"href") => {
                let reference = format!("&{};", e.decode()?);
                href.push_str(&quick_xml::escape::unescape(&reference)?);
            }
            Event::End(e) => {
                if e.local_name().as_ref() == b"href"
                    && path.len() >= 2
                    && path[path.len() - 2] == b"response"
                {
                    let url = base.join(&href)?;
                    if url.origin() == base.origin()
                        && url.query().is_none()
                        && url.fragment().is_none()
                        && let Some(name) = url.path().strip_prefix(base.path())
                        && !name.contains('/')
                        && let Some(id) = name.strip_suffix(".vsave")
                        && id.len() <= 64
                        && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
                        && backup::valid_component(id)
                    {
                        result.insert(id.to_owned());
                    }
                }
                path.pop();
            }
            Event::DocType(_) => anyhow::bail!("WebDAV XML DTD is not supported"),
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(result.into_iter().rev().map(|id| Remote { id }).collect())
}
struct ProgressReader<R> {
    inner: R,
    control: Control,
    done: u64,
    total: u64,
}
impl<R: Read> Read for ProgressReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.control.check().map_err(std::io::Error::other)?;
        let n = self.inner.read(buf)?;
        self.done += n as u64;
        self.control.set("upload", "", self.done, self.total);
        Ok(n)
    }
}
