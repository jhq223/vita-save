fn main() {
    if let Err(error) = run() {
        eprintln!("{error:#}");
        #[cfg(target_os = "vita")]
        {
            let root = std::path::Path::new("ux0:data/vita-save");
            let _ = std::fs::create_dir_all(root);
            let _ = std::fs::write(root.join("startup-error.log"), format!("{error:#}\n"));
        }
        std::process::exit(1);
    }
}
#[cfg(target_os = "vita")]
fn run() -> anyhow::Result<()> {
    vita_save::platform::run()
}

#[cfg(not(target_os = "vita"))]
fn run() -> anyhow::Result<()> {
    use vita_save::{backup::Store, job::Control, platform::Environment, saves};
    let args: Vec<String> = std::env::args().skip(1).collect();
    anyhow::ensure!(
        args.len() >= 2,
        "Usage: vita-save <list|backup|verify|export> <fixture-root> [save-id] [snapshot-id|archive-path]"
    );
    let env = Environment::host(std::path::Path::new(&args[1]));
    let control = Control::default();
    let store = Store::new(&env.data);
    let games = saves::scan(&env.save_roots, None, &control)?;
    if args[0] == "list" {
        for game in games {
            println!("{}\t{}\t{}", game.save_id, game.name, game.path.display());
        }
        return Ok(());
    }
    let id = args
        .get(2)
        .ok_or_else(|| anyhow::anyhow!("Missing save ID"))?;
    let game = games
        .iter()
        .find(|g| &g.save_id == id)
        .ok_or_else(|| anyhow::anyhow!("Save not found: {id}"))?;
    match args[0].as_str() {
        "backup" => {
            let m = vita_save::platform::with_save(game, |path, _| {
                store.create(game, path, false, &control)
            })?;
            println!("{}", m.id);
        }
        "verify" => {
            for m in store.list(game)? {
                store.verify(&m.id, &control)?;
                println!("{}\t{} bytes\tOK", m.id, m.bytes());
            }
        }
        "export" => {
            let id = args
                .get(3)
                .ok_or_else(|| anyhow::anyhow!("Missing snapshot ID"))?;
            let path = env.data.join(format!("{id}.vsave"));
            vita_save::cloud::archive::export(&store, id, std::fs::File::create(&path)?, &control)?;
            println!("{}", path.display());
        }
        _ => anyhow::bail!("Unknown command"),
    }
    Ok(())
}
