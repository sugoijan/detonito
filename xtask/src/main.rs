use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use roxmltree::{Document, Node};
use toml_edit::{DocumentMut, value};
use ttf_parser::{Face, OutlineBuilder};
use xmlwriter::{Indent, Options, XmlWriter};

const SVG_NS: &str = "http://www.w3.org/2000/svg";
const DEV_HOST: &str = "127.0.0.1";
const TRUNK_PORT: &str = "4361";
const CADDY_PORT: &str = "4365";
const WORKER_PORT: &str = "4377";
const WORKER_INSPECTOR_PORT: &str = "4388";
const GENERATED_CADDYFILE: &str = "dist/dev/Caddyfile.dev.generated";
const GENERATED_WRANGLER_CONFIG: &str = "wrangler.generated.toml";
const DEFAULT_BASE_PATH: &str = "/detonito";
const OPENMOJI_LAYER_IDS: &[&str] = &[
    "color",
    "color-foreground",
    "hair",
    "skin",
    "skin-shadow",
    "line",
    "line-supplement",
];
const OPENMOJI_PALETTE_STYLE: &str = r#"
.dtn-palette-fill-blue { fill: var(--dtn-sprite-blue); }
.dtn-palette-fill-blue-shade { fill: var(--dtn-sprite-blue-shade); }
.dtn-palette-fill-red { fill: var(--dtn-sprite-red); }
.dtn-palette-fill-red-shade { fill: var(--dtn-sprite-red-shade); }
.dtn-palette-fill-green { fill: var(--dtn-sprite-green); }
.dtn-palette-fill-green-shade { fill: var(--dtn-sprite-green-shade); }
.dtn-palette-fill-yellow { fill: var(--dtn-sprite-yellow); }
.dtn-palette-fill-yellow-shade { fill: var(--dtn-sprite-yellow-shade); }
.dtn-palette-fill-white { fill: var(--dtn-sprite-white); }
.dtn-palette-fill-gray-light { fill: var(--dtn-sprite-gray-light); }
.dtn-palette-fill-gray { fill: var(--dtn-sprite-gray); }
.dtn-palette-fill-gray-dark { fill: var(--dtn-sprite-gray-dark); }
.dtn-palette-fill-ink { fill: var(--dtn-sprite-ink); }
.dtn-palette-fill-pink { fill: var(--dtn-sprite-pink); }
.dtn-palette-fill-pink-shade { fill: var(--dtn-sprite-pink-shade); }
.dtn-palette-fill-purple { fill: var(--dtn-sprite-purple); }
.dtn-palette-fill-purple-shade { fill: var(--dtn-sprite-purple-shade); }
.dtn-palette-fill-orange { fill: var(--dtn-sprite-orange); }
.dtn-palette-fill-orange-shade { fill: var(--dtn-sprite-orange-shade); }
.dtn-palette-fill-brown { fill: var(--dtn-sprite-brown); }
.dtn-palette-fill-brown-shade { fill: var(--dtn-sprite-brown-shade); }
.dtn-palette-stroke-blue { stroke: var(--dtn-sprite-blue); }
.dtn-palette-stroke-blue-shade { stroke: var(--dtn-sprite-blue-shade); }
.dtn-palette-stroke-red { stroke: var(--dtn-sprite-red); }
.dtn-palette-stroke-red-shade { stroke: var(--dtn-sprite-red-shade); }
.dtn-palette-stroke-green { stroke: var(--dtn-sprite-green); }
.dtn-palette-stroke-green-shade { stroke: var(--dtn-sprite-green-shade); }
.dtn-palette-stroke-yellow { stroke: var(--dtn-sprite-yellow); }
.dtn-palette-stroke-yellow-shade { stroke: var(--dtn-sprite-yellow-shade); }
.dtn-palette-stroke-white { stroke: var(--dtn-sprite-white); }
.dtn-palette-stroke-gray-light { stroke: var(--dtn-sprite-gray-light); }
.dtn-palette-stroke-gray { stroke: var(--dtn-sprite-gray); }
.dtn-palette-stroke-gray-dark { stroke: var(--dtn-sprite-gray-dark); }
.dtn-palette-stroke-ink { stroke: var(--dtn-sprite-ink); }
.dtn-palette-stroke-pink { stroke: var(--dtn-sprite-pink); }
.dtn-palette-stroke-pink-shade { stroke: var(--dtn-sprite-pink-shade); }
.dtn-palette-stroke-purple { stroke: var(--dtn-sprite-purple); }
.dtn-palette-stroke-purple-shade { stroke: var(--dtn-sprite-purple-shade); }
.dtn-palette-stroke-orange { stroke: var(--dtn-sprite-orange); }
.dtn-palette-stroke-orange-shade { stroke: var(--dtn-sprite-orange-shade); }
.dtn-palette-stroke-brown { stroke: var(--dtn-sprite-brown); }
.dtn-palette-stroke-brown-shade { stroke: var(--dtn-sprite-brown-shade); }
.dtn-glyph-path { fill: currentColor; }"#;

#[derive(Parser)]
#[command(
    author,
    version,
    about = "Workspace asset task runner",
    propagate_version = true
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the local Trunk, Wrangler, and Caddy stack together behind one front door.
    #[command(name = "dev")]
    Dev,
    /// Print the local development ports and the preferred browser URL.
    #[command(name = "ports")]
    Ports,
    /// Run the standalone Trunk dev server without AFK mode.
    #[command(name = "web")]
    Web,
    /// Run the local Worker through Wrangler using the generated dev config.
    #[command(name = "worker", alias = "worker-dev")]
    WorkerDev,
    /// Build the Worker frontend bundle into its staged asset directory.
    #[command(name = "stage-assets")]
    StageAssets(StageAssetsArgs),
    /// Build the Worker release shim after staging the Worker frontend assets.
    #[command(name = "worker-build")]
    WorkerBuild,
    /// Build the Worker artifacts and generated Wrangler config for CI/CD deploys.
    #[command(name = "worker-prepare-deploy")]
    WorkerPrepareDeploy,
    /// Deploy the Worker using the generated release config.
    #[command(name = "worker-deploy")]
    WorkerDeploy,
    /// Run the local Caddy front door.
    #[command(name = "caddy")]
    Caddy,
    /// Copy curated upstream OpenMoji SVGs from a local checkout and rebuild the sprite.
    #[command(name = "sync-openmoji")]
    SyncOpenmoji(SyncOpenmojiArgs),
    /// Regenerate the combined sprite from committed source assets.
    #[command(name = "regen-sprite")]
    RegenSprite,
    /// Rebuild the local Iosevka-derived glyph sprite fragment and the combined sprite.
    #[command(name = "regen-fonts")]
    RegenFonts,
}

#[derive(Args)]
struct SyncOpenmojiArgs {
    /// Path to a local OpenMoji checkout. Defaults to ../openmoji.
    #[arg(long)]
    openmoji_dir: Option<PathBuf>,
}

#[derive(Args)]
struct StageAssetsArgs {
    /// Build the release asset bundle instead of the local dev bundle.
    #[arg(long)]
    release: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BuildProfile {
    Dev,
    Release,
}

#[derive(Clone, Debug)]
struct AssetPaths {
    repo_root: PathBuf,
    generated_dir: PathBuf,
    openmoji_manifest: PathBuf,
    openmoji_upstream_dir: PathBuf,
    openmoji_custom_dir: PathBuf,
    openmoji_symbols: PathBuf,
    iosevka_build_plan: PathBuf,
    iosevka_glyphs: PathBuf,
    sprite: PathBuf,
    local_iosevka_repo: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IconSource {
    Upstream,
    Custom,
}

#[derive(Clone, Debug)]
struct ManifestEntry {
    icon_name: String,
    source: IconSource,
    filename: String,
}

#[derive(Clone, Copy)]
struct GlyphSpec {
    id: &'static str,
    source: &'static str,
    chars: &'static str,
    view_box: (f32, f32),
}

const GLYPH_SPECS: &[GlyphSpec] = &[
    GlyphSpec {
        id: "counter",
        source: "IosevkaCustom-CondensedLight.ttf",
        chars: "-0123456789",
        view_box: (5.0, 9.0),
    },
    GlyphSpec {
        id: "cell",
        source: "IosevkaCustom-ExtendedHeavy.ttf",
        chars: "012345678",
        view_box: (4.0, 5.0),
    },
    GlyphSpec {
        id: "ui",
        source: "IosevkaCustom-Condensed.ttf",
        chars: "0123456789×",
        view_box: (5.0, 9.0),
    },
];

fn main() -> Result<()> {
    let cli = Cli::parse();
    let paths = workspace_paths()?;
    let root = paths.repo_root.clone();

    match cli.command {
        Commands::Dev => dev(&root),
        Commands::Ports => print_ports(&root),
        Commands::Web => spawn_web(&root, false)?
            .wait()
            .map(|_| ())
            .context("failed to wait for trunk"),
        Commands::WorkerDev => {
            ensure_dev_vars(&root)?;
            prepare_worker_runtime(&root, BuildProfile::Dev)?;
            let (mut worker, _) = spawn_worker_process(&root)?;
            worker
                .wait()
                .map(|_| ())
                .context("failed to wait for wrangler")
        }
        Commands::StageAssets(args) => stage_worker_assets(
            &root,
            if args.release {
                BuildProfile::Release
            } else {
                BuildProfile::Dev
            },
        ),
        Commands::WorkerBuild => {
            stage_worker_assets(&root, BuildProfile::Release)?;
            run_worker_build(&root, BuildProfile::Release)
        }
        Commands::WorkerPrepareDeploy => prepare_worker_deploy(&root, BuildProfile::Release),
        Commands::WorkerDeploy => {
            stage_worker_assets(&root, BuildProfile::Release)?;
            run_wrangler(&root, "deploy", BuildProfile::Release)
        }
        Commands::Caddy => spawn_caddy(&root)?
            .wait()
            .map(|_| ())
            .context("failed to wait for caddy"),
        Commands::SyncOpenmoji(args) => sync_openmoji(
            &paths,
            args.openmoji_dir
                .unwrap_or_else(|| paths.repo_root.parent().unwrap().join("openmoji")),
        ),
        Commands::RegenSprite => regen_sprite(&paths),
        Commands::RegenFonts => regen_fonts(&paths),
    }
}

fn workspace_paths() -> Result<AssetPaths> {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("xtask manifest dir has no parent")?
        .to_path_buf();
    let web_dir = repo_root.join("web");
    let generated_dir = web_dir.join("generated");

    Ok(AssetPaths {
        repo_root: repo_root.clone(),
        generated_dir: generated_dir.clone(),
        openmoji_manifest: web_dir.join("assets/openmoji/manifest.toml"),
        openmoji_upstream_dir: web_dir.join("assets/openmoji/upstream"),
        openmoji_custom_dir: web_dir.join("assets/openmoji/custom"),
        openmoji_symbols: generated_dir.join("openmoji-symbols.svg"),
        iosevka_build_plan: web_dir.join("assets/iosevka/private-build-plans.toml"),
        iosevka_glyphs: generated_dir.join("iosevka-glyphs.svg"),
        sprite: generated_dir.join("sprite.svg"),
        local_iosevka_repo: repo_root.parent().unwrap().join("Iosevka"),
    })
}

fn dev(root: &Path) -> Result<()> {
    ensure_dev_vars(root)?;
    prepare_worker_runtime(root, BuildProfile::Dev)?;

    let public_url = local_public_url(root)?;
    eprintln!("[xtask] Open {public_url}");
    eprintln!(
        "[xtask] Use the Caddy front door for manual QA. The raw Trunk and Wrangler ports are internal-only."
    );

    let (worker, worker_ready) = spawn_worker_process(root)?;
    let web = spawn_web(root, true)?;
    wait_for_worker_ready(worker_ready)?;
    let caddy = spawn_caddy(root)?;

    let mut children = vec![("worker", worker), ("web", web), ("caddy", caddy)];
    let code = wait_first_exit(&mut children);
    kill_all(&mut children);
    for (_, child) in &mut children {
        let _ = child.wait();
    }
    std::process::exit(code);
}

fn print_ports(root: &Path) -> Result<()> {
    let base_path = configured_base_path(root)?;
    println!("caddy:      http://localhost:{CADDY_PORT}");
    println!(
        "app:        http://localhost:{CADDY_PORT}{}  (use this)",
        trunk_public_url(&base_path)
    );
    println!("trunk:      http://{DEV_HOST}:{TRUNK_PORT}     (internal only)");
    println!("worker:     http://{DEV_HOST}:{WORKER_PORT}     (internal only)");
    println!("inspector:  http://{DEV_HOST}:{WORKER_INSPECTOR_PORT}");
    Ok(())
}

fn ensure_dev_vars(root: &Path) -> Result<()> {
    let target = worker_dir(root).join(".dev.vars");
    if target.exists() {
        return Ok(());
    }

    let example = worker_dir(root).join(".dev.vars.example");
    if example.exists() {
        fs::copy(&example, &target).with_context(|| {
            format!(
                "failed to initialize {} from {}",
                target.display(),
                example.display()
            )
        })?;
    } else {
        fs::write(
            &target,
            "AUTH_SIGNING_SECRET=detonito-local-dev-secret\nTWITCH_CLIENT_SECRET=replace-me\n",
        )
        .with_context(|| format!("failed to write {}", target.display()))?;
    }

    Ok(())
}

fn prepare_worker_runtime(root: &Path, profile: BuildProfile) -> Result<()> {
    stage_worker_assets(root, profile)?;
    write_generated_wrangler_config(root, profile)?;
    Ok(())
}

fn stage_worker_assets(root: &Path, profile: BuildProfile) -> Result<()> {
    let base_path = configured_base_path(root)?;
    let assets_root = worker_assets_root(root);
    let dist_dir = worker_dist_dir(root, &base_path);

    if assets_root.exists() {
        fs::remove_dir_all(&assets_root)
            .with_context(|| format!("failed to remove {}", assets_root.display()))?;
    }
    fs::create_dir_all(&dist_dir)
        .with_context(|| format!("failed to create {}", dist_dir.display()))?;

    let mut command = Command::new("trunk");
    command.arg("build");
    if profile == BuildProfile::Release {
        command.arg("--release");
    } else {
        command.args(["--cargo-profile", "web-dev"]);
    }
    command
        .args([
            "--dist",
            dist_dir
                .to_str()
                .context("invalid worker asset dist path")?,
            "--public-url",
            &trunk_public_url(&base_path),
        ])
        .env_remove("NO_COLOR")
        .current_dir(web_dir(root));
    run(&mut command)?;

    if base_path != "/" {
        fs::copy(dist_dir.join("index.html"), assets_root.join("index.html")).with_context(
            || {
                format!(
                    "failed to copy {} to {}",
                    dist_dir.join("index.html").display(),
                    assets_root.join("index.html").display()
                )
            },
        )?;
    }

    write_worker_headers(root, &base_path)
}

fn run_worker_build(root: &Path, profile: BuildProfile) -> Result<()> {
    let mut command = Command::new("worker-build");
    command.arg(match profile {
        BuildProfile::Dev => "--dev",
        BuildProfile::Release => "--release",
    });
    command.arg(".").current_dir(worker_dir(root));
    run(&mut command)
}

fn prepare_worker_deploy(root: &Path, profile: BuildProfile) -> Result<()> {
    stage_worker_assets(root, profile)?;
    run_worker_build(root, profile)?;
    write_generated_wrangler_config(root, profile)?;
    Ok(())
}

fn write_generated_wrangler_config(root: &Path, profile: BuildProfile) -> Result<PathBuf> {
    let template_path = worker_dir(root).join("wrangler.toml");
    let template = fs::read_to_string(&template_path)
        .with_context(|| format!("failed to read {}", template_path.display()))?;
    let mut document = template
        .parse::<DocumentMut>()
        .with_context(|| format!("failed to parse {}", template_path.display()))?;

    let base_path = configured_base_path(root)?;
    match profile {
        BuildProfile::Dev => {
            document["build"]["command"] = value("worker-build --dev .");
        }
        BuildProfile::Release => {
            document.remove("build");
        }
    }
    document["assets"]["directory"] = value("./build/assets");
    document["assets"]["binding"] = value("ASSETS");
    document["assets"]["not_found_handling"] = value("single-page-application");
    let mut run_worker_first = toml_edit::Array::default();
    for pattern in run_worker_first_patterns(&base_path) {
        run_worker_first.push(pattern);
    }
    document["assets"]["run_worker_first"] = value(run_worker_first);
    document["vars"]["BASE_PATH"] = value(base_path.clone());
    if profile == BuildProfile::Dev {
        document["vars"]["PUBLIC_URL"] = value(local_public_url(root)?);
    }
    if profile == BuildProfile::Release {
        if let Some(host) = release_route_host(&document) {
            let mut routes = toml_edit::Array::default();
            for pattern in worker_route_patterns(&host, &base_path) {
                routes.push(pattern);
            }
            document["routes"] = value(routes);
        }
    }

    let output_path = worker_dir(root).join(GENERATED_WRANGLER_CONFIG);
    fs::write(&output_path, document.to_string())
        .with_context(|| format!("failed to write {}", output_path.display()))?;
    Ok(output_path)
}

fn release_route_host(document: &DocumentMut) -> Option<String> {
    env::var("WORKER_ROUTE_HOST")
        .ok()
        .map(|host| host.trim().trim_end_matches('/').to_string())
        .filter(|host| !host.is_empty())
        .or_else(|| public_url_host(document["vars"]["PUBLIC_URL"].as_str().unwrap_or_default()))
}

fn public_url_host(public_url: &str) -> Option<String> {
    let trimmed = public_url.trim();
    if trimmed.is_empty() {
        return None;
    }

    let without_scheme = trimmed
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(trimmed);
    let host = without_scheme
        .split('/')
        .next()
        .unwrap_or_default()
        .trim()
        .trim_end_matches('/');

    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

fn run_wrangler(root: &Path, subcommand: &str, profile: BuildProfile) -> Result<()> {
    let config_path = write_generated_wrangler_config(root, profile)?;
    let mut command = wrangler_command(root)?;
    command.args([
        subcommand,
        "--config",
        config_path
            .file_name()
            .and_then(|file_name| file_name.to_str())
            .context("invalid generated wrangler config path")?,
    ]);
    run(&mut command)
}

fn spawn_worker_process(root: &Path) -> Result<(Child, mpsc::Receiver<()>)> {
    let config_path = write_generated_wrangler_config(root, BuildProfile::Dev)?;
    let mut command = wrangler_command(root)?;
    command
        .args([
            "dev",
            "--config",
            config_path
                .file_name()
                .and_then(|file_name| file_name.to_str())
                .context("invalid generated wrangler config path")?,
            "--ip",
            DEV_HOST,
            "--port",
            WORKER_PORT,
            "--inspector-port",
            WORKER_INSPECTOR_PORT,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().context("failed to start wrangler dev")?;

    let stdout = child
        .stdout
        .take()
        .context("worker stdout was not captured")?;
    let stderr = child
        .stderr
        .take()
        .context("worker stderr was not captured")?;
    let (ready_tx, ready_rx) = mpsc::channel::<()>();

    relay_process_output(stdout, ready_tx.clone());
    relay_process_output(stderr, ready_tx);

    Ok((child, ready_rx))
}

fn spawn_web(root: &Path, afk_enabled: bool) -> Result<Child> {
    let base_path = configured_base_path(root)?;
    let mut command = Command::new("trunk");
    command.args([
        "serve",
        "--address",
        DEV_HOST,
        "--port",
        TRUNK_PORT,
        "--cargo-profile",
        "web-dev",
        "--public-url",
        &trunk_public_url(&base_path),
        "--serve-base",
        &trunk_public_url(&base_path),
    ]);
    if !afk_enabled {
        command.args(["--no-default-features", "--features", "web-static"]);
    }
    command
        .env_remove("NO_COLOR")
        .current_dir(web_dir(root))
        .spawn()
        .context("failed to start trunk serve")
}

fn wait_for_worker_ready(rx: mpsc::Receiver<()>) -> Result<()> {
    rx.recv_timeout(Duration::from_secs(30))
        .map_err(|_| anyhow::anyhow!("timed out waiting for worker to become ready"))
}

fn relay_process_output<R>(stream: R, ready_tx: mpsc::Sender<()>)
where
    R: std::io::Read + Send + 'static,
{
    thread::spawn(move || {
        let reader = BufReader::new(stream);
        let mut sent_ready = false;
        for line in reader.lines() {
            let Ok(line) = line else {
                break;
            };
            eprintln!("{line}");
            if !sent_ready && line.contains("Ready on http://") {
                let _ = ready_tx.send(());
                sent_ready = true;
            }
        }
    });
}

fn write_generated_caddyfile(root: &Path) -> Result<PathBuf> {
    let base_path = configured_base_path(root)?;
    let app_public_url = trunk_public_url(&base_path);
    let worker_paths = [
        join_base_path(&base_path, "/healthz"),
        join_base_path(&base_path, "/ws/*"),
        join_base_path(&base_path, "/api/*"),
        join_base_path(&base_path, "/auth/*"),
    ]
    .join(" ");

    let mut content = format!(
        "{{\n\tservers 127.0.0.1:{CADDY_PORT} {{\n\t\tprotocols h1 h2c\n\t}}\n\tservers [::1]:{CADDY_PORT} {{\n\t\tprotocols h1 h2c\n\t}}\n\tauto_https off\n\tadmin off\n}}\n\n"
    );
    content.push_str(&format!(":{CADDY_PORT} {{\n\tbind 127.0.0.1 ::1\n"));
    if base_path != "/" {
        content.push_str(&format!("\tredir / {app_public_url} 302\n"));
    }
    content.push_str(&format!("\t@worker path {worker_paths}\n"));
    content.push_str(&format!(
        "\thandle @worker {{\n\t\treverse_proxy {DEV_HOST}:{WORKER_PORT}\n\t}}\n"
    ));
    if base_path == "/" {
        content.push_str(&format!(
            "\thandle {{\n\t\treverse_proxy {DEV_HOST}:{TRUNK_PORT}\n\t}}\n"
        ));
    } else {
        content.push_str(&format!(
            "\t@app path {} {} {}\n",
            base_path,
            join_base_path(&base_path, "/"),
            join_base_path(&base_path, "/*")
        ));
        content.push_str(&format!(
            "\thandle @app {{\n\t\treverse_proxy {DEV_HOST}:{TRUNK_PORT}\n\t}}\n\thandle {{\n\t\trespond \"not found\" 404\n\t}}\n"
        ));
    }
    content.push_str("}\n");

    let output_path = root.join(GENERATED_CADDYFILE);
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(&output_path, content)
        .with_context(|| format!("failed to write {}", output_path.display()))?;
    Ok(output_path)
}

fn spawn_caddy(root: &Path) -> Result<Child> {
    let config_path = write_generated_caddyfile(root)?;
    Command::new("caddy")
        .args([
            "run",
            "--config",
            config_path
                .to_str()
                .context("invalid generated caddy config path")?,
            "--adapter",
            "caddyfile",
        ])
        .current_dir(root)
        .spawn()
        .context("failed to start caddy")
}

fn wait_first_exit(children: &mut Vec<(&str, Child)>) -> i32 {
    loop {
        for (name, child) in children.iter_mut() {
            if let Ok(Some(status)) = child.try_wait() {
                let code = status.code().unwrap_or(1);
                eprintln!("[xtask] '{name}' exited with status {status}");
                return code;
            }
        }
        thread::sleep(Duration::from_millis(250));
    }
}

fn kill_all(children: &mut Vec<(&str, Child)>) {
    for (_, child) in children.iter_mut() {
        let _ = child.kill();
    }
}

fn configured_base_path(root: &Path) -> Result<String> {
    let template_path = worker_dir(root).join("wrangler.toml");
    let template = fs::read_to_string(&template_path)
        .with_context(|| format!("failed to read {}", template_path.display()))?;
    let document = template
        .parse::<DocumentMut>()
        .with_context(|| format!("failed to parse {}", template_path.display()))?;
    Ok(normalize_base_path(
        document["vars"]["BASE_PATH"]
            .as_str()
            .unwrap_or(DEFAULT_BASE_PATH),
    ))
}

fn local_public_origin() -> String {
    format!("http://localhost:{CADDY_PORT}")
}

fn local_public_url(root: &Path) -> Result<String> {
    Ok(prefixed_public_url(
        &configured_base_path(root)?,
        &local_public_origin(),
    ))
}

fn prefixed_public_url(base_path: &str, origin: &str) -> String {
    let origin = origin.trim_end_matches('/');
    if base_path == "/" {
        origin.to_string()
    } else {
        format!("{origin}{base_path}")
    }
}

fn trunk_public_url(base_path: &str) -> String {
    if base_path == "/" {
        "/".to_string()
    } else {
        format!("{base_path}/")
    }
}

fn normalize_base_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed == "/" {
        return "/".to_string();
    }
    let normalized = trimmed.trim_end_matches('/');
    if normalized.starts_with('/') {
        normalized.to_string()
    } else {
        format!("/{normalized}")
    }
}

fn worker_route_patterns(host: &str, base_path: &str) -> Vec<String> {
    let host = host.trim().trim_end_matches('/');
    if base_path == "/" {
        vec![format!("{host}/*")]
    } else {
        vec![format!("{host}{base_path}"), format!("{host}{base_path}/*")]
    }
}

fn run_worker_first_patterns(base_path: &str) -> Vec<String> {
    ["/healthz", "/api/*", "/auth/*", "/ws/*"]
        .into_iter()
        .map(|path| join_base_path(base_path, path))
        .collect()
}

fn write_worker_headers(root: &Path, base_path: &str) -> Result<()> {
    let assets_root = worker_assets_root(root);
    let root_route = if base_path == "/" {
        "/".to_string()
    } else {
        format!("{base_path}/")
    };
    let index_route = if base_path == "/" {
        "/index.html".to_string()
    } else {
        format!("{base_path}/index.html")
    };
    let css_route = scoped_route(base_path, "*.css");
    let js_route = scoped_route(base_path, "*.js");
    let wasm_route = scoped_route(base_path, "*.wasm");
    let svg_route = scoped_route(base_path, "*.svg");

    let content = format!(
        "{root_route}\n  Cache-Control: public, max-age=0, must-revalidate\n\n{index_route}\n  Cache-Control: public, max-age=0, must-revalidate\n\n{css_route}\n  Cache-Control: public, max-age=31536000, immutable\n\n{js_route}\n  Cache-Control: public, max-age=31536000, immutable\n\n{wasm_route}\n  Cache-Control: public, max-age=31536000, immutable\n\n{svg_route}\n  Cache-Control: public, max-age=31536000, immutable\n"
    );
    fs::write(assets_root.join("_headers"), content)
        .with_context(|| format!("failed to write {}", assets_root.join("_headers").display()))
}

fn scoped_route(base_path: &str, suffix: &str) -> String {
    if base_path == "/" {
        format!("/{suffix}")
    } else {
        format!("{base_path}/{suffix}")
    }
}

fn join_base_path(base_path: &str, path: &str) -> String {
    let base_path = normalize_base_path(base_path);
    let path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };

    if base_path == "/" {
        path
    } else if path == "/" {
        format!("{base_path}/")
    } else {
        format!("{base_path}{path}")
    }
}

fn web_dir(root: &Path) -> PathBuf {
    root.join("web")
}

fn worker_dir(root: &Path) -> PathBuf {
    root.join("worker")
}

fn worker_assets_root(root: &Path) -> PathBuf {
    root.join("worker/build/assets")
}

fn worker_dist_dir(root: &Path, base_path: &str) -> PathBuf {
    let assets_root = worker_assets_root(root);
    if base_path == "/" {
        assets_root
    } else {
        assets_root.join(base_path.trim_start_matches('/'))
    }
}

fn wrangler_command(root: &Path) -> Result<Command> {
    if command_exists("wrangler") {
        let mut command = Command::new("wrangler");
        command.current_dir(worker_dir(root));
        return Ok(command);
    }

    if let Some(local) = local_wrangler_binary(root) {
        let mut command = Command::new(local);
        command.current_dir(worker_dir(root));
        return Ok(command);
    }

    if command_exists("pnpm") && worker_dir(root).join("package.json").exists() {
        let mut command = Command::new("pnpm");
        command
            .args(["exec", "wrangler"])
            .current_dir(worker_dir(root));
        return Ok(command);
    }

    if command_exists("npm") && worker_dir(root).join("package.json").exists() {
        let mut command = Command::new("npm");
        command
            .args(["exec", "--", "wrangler"])
            .current_dir(worker_dir(root));
        return Ok(command);
    }

    if command_exists("npx") {
        let mut command = Command::new("npx");
        command.args(["wrangler"]).current_dir(worker_dir(root));
        return Ok(command);
    }

    bail!(
        "Wrangler CLI was not found. Install it globally or run `cd worker && pnpm install` (or `npm install`) to install the local dev dependency."
    );
}

fn local_wrangler_binary(root: &Path) -> Option<PathBuf> {
    let bin_dir = worker_dir(root).join("node_modules/.bin");
    [bin_dir.join("wrangler"), bin_dir.join("wrangler.cmd")]
        .into_iter()
        .find(|path| path.is_file())
}

fn command_exists(command: &str) -> bool {
    Command::new(command)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

fn sync_openmoji(paths: &AssetPaths, openmoji_dir: PathBuf) -> Result<()> {
    if !openmoji_dir.is_dir() {
        bail!(
            "OpenMoji directory does not exist: {}",
            openmoji_dir.display()
        );
    }

    refresh_openmoji_provenance(paths, &openmoji_dir)?;
    let manifest = load_openmoji_manifest(paths)?;
    fs::create_dir_all(&paths.openmoji_upstream_dir)
        .with_context(|| format!("failed to create {}", paths.openmoji_upstream_dir.display()))?;

    let mut expected = BTreeSet::new();
    for entry in &manifest {
        if entry.source != IconSource::Upstream {
            continue;
        }

        let source_path = openmoji_dir.join("color/svg").join(&entry.filename);
        if !source_path.is_file() {
            bail!(
                "OpenMoji asset for {:?} not found at {}",
                entry.icon_name,
                source_path.display()
            );
        }

        let destination = paths.openmoji_upstream_dir.join(&entry.filename);
        let source_svg = fs::read_to_string(&source_path)
            .with_context(|| format!("failed to read {}", source_path.display()))?;
        let normalized_svg = normalize_openmoji_imported_svg(&source_svg)
            .with_context(|| format!("failed to normalize {}", source_path.display()))?;
        fs::write(&destination, normalized_svg).with_context(|| {
            format!(
                "failed to write normalized OpenMoji asset {}",
                destination.display()
            )
        })?;
        expected.insert(entry.filename.clone());
    }

    for existing in fs::read_dir(&paths.openmoji_upstream_dir)
        .with_context(|| format!("failed to read {}", paths.openmoji_upstream_dir.display()))?
    {
        let existing = existing?;
        let path = existing.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("svg")
            && !expected.contains(
                path.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default(),
            )
        {
            fs::remove_file(&path)
                .with_context(|| format!("failed to remove {}", path.display()))?;
        }
    }

    write_openmoji_symbols(paths, &manifest)?;
    write_combined_sprite(paths)
}

fn normalize_openmoji_imported_svg(svg_source: &str) -> Result<String> {
    let document = Document::parse(svg_source).context("failed to parse imported OpenMoji SVG")?;
    let root = document.root_element();
    let mut writer = new_xml_writer();
    write_normalized_openmoji_imported_node(&mut writer, root, AffineTransform::identity())?;
    Ok(writer.end_document())
}

fn write_normalized_openmoji_imported_node(
    writer: &mut XmlWriter,
    node: Node<'_, '_>,
    inherited_transform: AffineTransform,
) -> Result<()> {
    let tag = node.tag_name().name();
    let mut attrs: BTreeMap<String, String> = node
        .attributes()
        .filter(|attribute| attribute.namespace().is_none())
        .map(|attribute| (attribute.name().to_string(), attribute.value().to_string()))
        .collect();
    let local_transform = attrs
        .remove("transform")
        .map(|value| parse_svg_transform(&value))
        .transpose()?
        .unwrap_or_else(AffineTransform::identity);
    let effective_transform = inherited_transform.compose(local_transform);

    match tag {
        "svg" | "g" => {
            writer.start_element(tag);
            if tag == "svg" {
                writer.write_attribute("xmlns", SVG_NS);
            }
            for (key, value) in &attrs {
                writer.write_attribute(key, value);
            }
            for child in node.children().filter(|child| child.is_element()) {
                write_normalized_openmoji_imported_node(writer, child, effective_transform)?;
            }
            writer.end_element();
            Ok(())
        }
        "ellipse" => {
            if effective_transform.is_identity() {
                writer.start_element("ellipse");
                for (key, value) in &attrs {
                    writer.write_attribute(key, value);
                }
                writer.end_element();
                Ok(())
            } else {
                write_flattened_ellipse_node(writer, &attrs, effective_transform)
            }
        }
        "circle" => {
            if effective_transform.is_identity() {
                writer.start_element("circle");
                for (key, value) in &attrs {
                    writer.write_attribute(key, value);
                }
                writer.end_element();
                Ok(())
            } else {
                write_flattened_circle_node(writer, &attrs, effective_transform)
            }
        }
        _ if effective_transform.is_identity() => {
            writer.start_element(tag);
            for (key, value) in &attrs {
                writer.write_attribute(key, value);
            }
            for child in node.children().filter(|child| child.is_element()) {
                write_normalized_openmoji_imported_node(
                    writer,
                    child,
                    AffineTransform::identity(),
                )?;
            }
            writer.end_element();
            Ok(())
        }
        _ => bail!(
            "unsupported non-identity transform on imported OpenMoji <{}> element",
            tag
        ),
    }
}

fn regen_sprite(paths: &AssetPaths) -> Result<()> {
    let manifest = load_openmoji_manifest(paths)?;
    write_openmoji_symbols(paths, &manifest)?;
    write_combined_sprite(paths)
}

fn refresh_openmoji_provenance(paths: &AssetPaths, openmoji_dir: &std::path::Path) -> Result<()> {
    let manifest_text = fs::read_to_string(&paths.openmoji_manifest)
        .with_context(|| format!("failed to read {}", paths.openmoji_manifest.display()))?;
    let mut document = manifest_text
        .parse::<DocumentMut>()
        .with_context(|| format!("failed to parse {}", paths.openmoji_manifest.display()))?;

    let repo_url = normalize_git_remote_url(&run_capture(
        Command::new("git")
            .args(["remote", "get-url", "origin"])
            .current_dir(openmoji_dir),
    )?);
    let commit = run_capture(
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(openmoji_dir),
    )?;
    let release = run_capture_optional(
        Command::new("git")
            .args(["describe", "--tags", "--abbrev=0"])
            .current_dir(openmoji_dir),
    )?;
    let describe = run_capture(
        Command::new("git")
            .args(["describe", "--tags", "--always", "--dirty"])
            .current_dir(openmoji_dir),
    )?;
    let dirty = !run_capture(
        Command::new("git")
            .args(["status", "--porcelain", "--untracked-files=no"])
            .current_dir(openmoji_dir),
    )?
    .is_empty();

    if !document["upstream"].is_table() {
        document["upstream"] = toml_edit::table();
    }
    document["upstream"]["project"] = value("OpenMoji");
    document["upstream"]["repository"] = value(repo_url);
    document["upstream"]["license"] = value("CC-BY-SA-4.0");
    document["upstream"]["source_checkout"] = value(openmoji_dir.display().to_string());
    if let Some(release) = release {
        document["upstream"]["release"] = value(release);
    }
    document["upstream"]["describe"] = value(describe);
    document["upstream"]["commit"] = value(commit);
    document["upstream"]["dirty"] = value(dirty);

    fs::write(&paths.openmoji_manifest, document.to_string())
        .with_context(|| format!("failed to write {}", paths.openmoji_manifest.display()))?;
    Ok(())
}

fn regen_fonts(paths: &AssetPaths) -> Result<()> {
    if !paths.local_iosevka_repo.is_dir() {
        bail!(
            "Expected a local Iosevka checkout at {}. This repository does not clone Iosevka automatically.",
            paths.local_iosevka_repo.display()
        );
    }
    if !paths.iosevka_build_plan.is_file() {
        bail!(
            "Missing Iosevka build plan: {}",
            paths.iosevka_build_plan.display()
        );
    }

    let repo_plan = paths.local_iosevka_repo.join("private-build-plans.toml");
    fs::copy(&paths.iosevka_build_plan, &repo_plan).with_context(|| {
        format!(
            "failed to copy {} to {}",
            paths.iosevka_build_plan.display(),
            repo_plan.display()
        )
    })?;

    if !paths.local_iosevka_repo.join("node_modules").is_dir() {
        run(Command::new("npm")
            .arg("install")
            .current_dir(&paths.local_iosevka_repo))?;
    }

    let mut build = Command::new("npm");
    build
        .args(["run", "build", "--", "woff2-unhinted::IosevkaCustom"])
        .current_dir(&paths.local_iosevka_repo);
    let status = run_allow_failure(&mut build)?;
    if !status.success() {
        run(Command::new("npm")
            .arg("install")
            .current_dir(&paths.local_iosevka_repo))?;
        run(Command::new("npm")
            .args(["run", "build", "--", "woff2-unhinted::IosevkaCustom"])
            .current_dir(&paths.local_iosevka_repo))?;
    }

    write_iosevka_glyphs(paths)?;
    write_openmoji_symbols(paths, &load_openmoji_manifest(paths)?)?;
    write_combined_sprite(paths)
}

fn load_openmoji_manifest(paths: &AssetPaths) -> Result<Vec<ManifestEntry>> {
    let manifest_text = fs::read_to_string(&paths.openmoji_manifest)
        .with_context(|| format!("failed to read {}", paths.openmoji_manifest.display()))?;
    let manifest_value: toml::Value = toml::from_str(&manifest_text)
        .with_context(|| format!("failed to parse {}", paths.openmoji_manifest.display()))?;
    let icons = manifest_value
        .get("icons")
        .and_then(toml::Value::as_table)
        .context("Expected [icons] entries in OpenMoji manifest")?;

    let mut manifest = Vec::new();
    for (icon_name, icon_value) in icons {
        let icon_table = icon_value
            .as_table()
            .with_context(|| format!("Expected [icons.{icon_name}] to be a table"))?;
        let source = match icon_table
            .get("source")
            .and_then(toml::Value::as_str)
            .unwrap_or_default()
        {
            "upstream" => IconSource::Upstream,
            "custom" => IconSource::Custom,
            other => bail!(
                "Icon {:?} must declare source = 'upstream' or source = 'custom', got {:?}",
                icon_name,
                other
            ),
        };
        let filename_key = match source {
            IconSource::Upstream => "upstream",
            IconSource::Custom => "custom",
        };
        let filename = icon_table
            .get(filename_key)
            .and_then(toml::Value::as_str)
            .with_context(|| format!("Icon {:?} must define {}", icon_name, filename_key))?;
        if !filename.ends_with(".svg") {
            bail!("Icon {:?} must point at an .svg file", icon_name);
        }
        manifest.push(ManifestEntry {
            icon_name: icon_name.to_string(),
            source,
            filename: filename.to_string(),
        });
    }

    Ok(manifest)
}

fn write_openmoji_symbols(paths: &AssetPaths, manifest: &[ManifestEntry]) -> Result<()> {
    ensure_generated_dir(paths)?;

    let mut writer = new_xml_writer();
    writer.start_element("svg");
    writer.write_attribute("xmlns", SVG_NS);
    writer.start_element("defs");

    for entry in manifest {
        let source_path = match entry.source {
            IconSource::Upstream => paths.openmoji_upstream_dir.join(&entry.filename),
            IconSource::Custom => paths.openmoji_custom_dir.join(&entry.filename),
        };
        let svg_source = fs::read_to_string(&source_path)
            .with_context(|| format!("failed to read {}", source_path.display()))?;
        let document = Document::parse(&svg_source)
            .with_context(|| format!("failed to parse {}", source_path.display()))?;
        let root = document.root_element();
        let view_box = root
            .attribute("viewBox")
            .with_context(|| format!("OpenMoji icon {:?} is missing a viewBox", entry.icon_name))?;

        writer.start_element("symbol");
        writer.write_attribute("id", &format!("dtn-icon-{}", entry.icon_name));
        writer.write_attribute("viewBox", view_box);

        for child in root.children().filter(|child| child.is_element()) {
            write_normalized_openmoji_node(&mut writer, child, &entry.icon_name, None)?;
        }

        writer.end_element();
    }

    writer.end_element();
    writer.end_element();

    fs::write(&paths.openmoji_symbols, writer.end_document())
        .with_context(|| format!("failed to write {}", paths.openmoji_symbols.display()))?;
    println!("Wrote {}", paths.openmoji_symbols.display());
    Ok(())
}

fn write_normalized_openmoji_node(
    writer: &mut XmlWriter,
    node: Node<'_, '_>,
    icon_name: &str,
    current_layer: Option<&str>,
) -> Result<()> {
    let tag = node.tag_name().name();
    if matches!(
        tag,
        "defs" | "desc" | "metadata" | "namedview" | "style" | "title"
    ) {
        return Ok(());
    }

    let mut raw_attrs: BTreeMap<String, String> = node
        .attributes()
        .filter(|attribute| attribute.namespace().is_none())
        .map(|attribute| (attribute.name().to_string(), attribute.value().to_string()))
        .collect();
    let mut style_attrs = parse_style(raw_attrs.remove("style").as_deref())?;
    for paint_attr in ["fill", "stroke"] {
        if !raw_attrs.contains_key(paint_attr) {
            if let Some(value) = style_attrs.remove(paint_attr) {
                raw_attrs.insert(paint_attr.to_string(), value);
            }
        }
    }

    let mut layer_name = current_layer.map(ToOwned::to_owned);
    if let Some(element_id) = raw_attrs.remove("id") {
        if OPENMOJI_LAYER_IDS.contains(&element_id.as_str()) {
            layer_name = Some(element_id);
        }
    }

    let mut attrs = BTreeMap::new();
    for (key, value) in raw_attrs.iter() {
        if key == "fill" || key == "stroke" {
            continue;
        }
        attrs.insert(key.clone(), value.clone());
    }
    for (key, value) in style_attrs {
        attrs.insert(key, value);
    }

    if let Some(layer_name) = layer_name.as_deref() {
        append_class(&mut attrs, "dtn-openmoji-layer");
        append_class(&mut attrs, &format!("dtn-openmoji-layer-{layer_name}"));
    }

    let fill = raw_attrs.get("fill").cloned();
    let stroke = raw_attrs.get("stroke").cloned();
    if let Some(fill) = fill {
        rewrite_paint(&mut attrs, icon_name, "fill", &fill)?;
    } else if layer_name.as_deref() == Some("line")
        && stroke.is_none()
        && matches!(
            tag,
            "circle" | "ellipse" | "path" | "polygon" | "polyline" | "rect"
        )
    {
        rewrite_paint(&mut attrs, icon_name, "fill", "#000000")?;
    }
    if let Some(stroke) = stroke {
        rewrite_paint(&mut attrs, icon_name, "stroke", &stroke)?;
    }

    writer.start_element(tag);
    for (key, value) in &attrs {
        writer.write_attribute(key, value);
    }
    for child in node.children() {
        if child.is_element() {
            write_normalized_openmoji_node(writer, child, icon_name, layer_name.as_deref())?;
        }
    }
    writer.end_element();
    Ok(())
}

fn parse_style(style: Option<&str>) -> Result<BTreeMap<String, String>> {
    let mut parsed = BTreeMap::new();
    let Some(style) = style else {
        return Ok(parsed);
    };

    for declaration in style.split(';') {
        let declaration = declaration.trim();
        if declaration.is_empty() {
            continue;
        }
        let Some((key, value)) = declaration.split_once(':') else {
            bail!("Could not parse style declaration {:?}", declaration);
        };
        parsed.insert(key.trim().to_string(), value.trim().to_string());
    }

    Ok(parsed)
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct AffineTransform {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    e: f64,
    f: f64,
}

impl AffineTransform {
    fn identity() -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        }
    }

    fn compose(self, other: Self) -> Self {
        Self {
            a: self.a * other.a + self.c * other.b,
            b: self.b * other.a + self.d * other.b,
            c: self.a * other.c + self.c * other.d,
            d: self.b * other.c + self.d * other.d,
            e: self.a * other.e + self.c * other.f + self.e,
            f: self.b * other.e + self.d * other.f + self.f,
        }
    }

    fn apply(self, x: f64, y: f64) -> (f64, f64) {
        (
            (self.a * x) + (self.c * y) + self.e,
            (self.b * x) + (self.d * y) + self.f,
        )
    }

    fn is_identity(self) -> bool {
        approx_eq(self.a, 1.0)
            && approx_eq(self.b, 0.0)
            && approx_eq(self.c, 0.0)
            && approx_eq(self.d, 1.0)
            && approx_eq(self.e, 0.0)
            && approx_eq(self.f, 0.0)
    }

    fn scale_x(self) -> f64 {
        self.a.hypot(self.b)
    }

    fn scale_y(self) -> f64 {
        self.c.hypot(self.d)
    }

    fn is_orthogonal(self) -> bool {
        approx_eq((self.a * self.c) + (self.b * self.d), 0.0)
    }

    fn determinant(self) -> f64 {
        (self.a * self.d) - (self.b * self.c)
    }

    fn rotation_degrees(self) -> f64 {
        self.b.atan2(self.a).to_degrees()
    }
}

fn approx_eq(left: f64, right: f64) -> bool {
    (left - right).abs() <= 0.001
}

fn parse_svg_transform(value: &str) -> Result<AffineTransform> {
    let mut remainder = value.trim();
    let mut transform = AffineTransform::identity();

    while !remainder.is_empty() {
        let open = remainder
            .find('(')
            .with_context(|| format!("invalid SVG transform {:?}", value))?;
        let close = remainder[open + 1..]
            .find(')')
            .map(|index| index + open + 1)
            .with_context(|| format!("invalid SVG transform {:?}", value))?;
        let name = remainder[..open].trim();
        let args = parse_transform_numbers(&remainder[open + 1..close])?;
        let next = match name {
            "matrix" => {
                if args.len() != 6 {
                    bail!("matrix transform requires 6 values, got {:?}", args);
                }
                AffineTransform {
                    a: args[0],
                    b: args[1],
                    c: args[2],
                    d: args[3],
                    e: args[4],
                    f: args[5],
                }
            }
            "translate" => {
                if !(1..=2).contains(&args.len()) {
                    bail!("translate transform requires 1 or 2 values, got {:?}", args);
                }
                AffineTransform {
                    e: args[0],
                    f: args.get(1).copied().unwrap_or(0.0),
                    ..AffineTransform::identity()
                }
            }
            "scale" => {
                if !(1..=2).contains(&args.len()) {
                    bail!("scale transform requires 1 or 2 values, got {:?}", args);
                }
                let sy = args.get(1).copied().unwrap_or(args[0]);
                AffineTransform {
                    a: args[0],
                    d: sy,
                    ..AffineTransform::identity()
                }
            }
            "rotate" => parse_rotate_transform(&args)?,
            other => bail!("unsupported SVG transform function {:?}", other),
        };
        transform = transform.compose(next);
        remainder = remainder[close + 1..]
            .trim_start_matches(|ch: char| ch.is_ascii_whitespace() || ch == ',');
    }

    Ok(transform)
}

fn parse_rotate_transform(args: &[f64]) -> Result<AffineTransform> {
    if !(1..=3).contains(&args.len()) {
        bail!("rotate transform requires 1 or 3 values, got {:?}", args);
    }
    let angle = args[0].to_radians();
    let rotation = AffineTransform {
        a: angle.cos(),
        b: angle.sin(),
        c: -angle.sin(),
        d: angle.cos(),
        ..AffineTransform::identity()
    };
    if args.len() == 1 {
        return Ok(rotation);
    }

    let translate_to_origin = AffineTransform {
        e: -args[1],
        f: -args[2],
        ..AffineTransform::identity()
    };
    let translate_back = AffineTransform {
        e: args[1],
        f: args[2],
        ..AffineTransform::identity()
    };
    Ok(translate_back
        .compose(rotation)
        .compose(translate_to_origin))
}

fn parse_transform_numbers(value: &str) -> Result<Vec<f64>> {
    value
        .split(|ch: char| ch == ',' || ch.is_ascii_whitespace())
        .filter(|part| !part.is_empty())
        .map(|part| {
            part.parse::<f64>()
                .with_context(|| format!("invalid transform number {:?}", part))
        })
        .collect()
}

fn write_flattened_circle_node(
    writer: &mut XmlWriter,
    attrs: &BTreeMap<String, String>,
    transform: AffineTransform,
) -> Result<()> {
    let cx = required_svg_number(attrs, "cx")?;
    let cy = required_svg_number(attrs, "cy")?;
    let r = required_svg_number(attrs, "r")?;
    write_flattened_elliptic_arc_path(writer, attrs, cx, cy, r, r, transform)
}

fn write_flattened_ellipse_node(
    writer: &mut XmlWriter,
    attrs: &BTreeMap<String, String>,
    transform: AffineTransform,
) -> Result<()> {
    let cx = required_svg_number(attrs, "cx")?;
    let cy = required_svg_number(attrs, "cy")?;
    let rx = required_svg_number(attrs, "rx")?;
    let ry = required_svg_number(attrs, "ry")?;
    write_flattened_elliptic_arc_path(writer, attrs, cx, cy, rx, ry, transform)
}

fn write_flattened_elliptic_arc_path(
    writer: &mut XmlWriter,
    attrs: &BTreeMap<String, String>,
    cx: f64,
    cy: f64,
    rx: f64,
    ry: f64,
    transform: AffineTransform,
) -> Result<()> {
    if !transform.is_orthogonal() || transform.determinant() <= 0.0 {
        bail!(
            "cannot flatten non-orthogonal ellipse transform {:?}",
            transform
        );
    }

    let scale_x = transform.scale_x();
    let scale_y = transform.scale_y();
    let rx = rx * scale_x;
    let ry = ry * scale_y;
    let rotation = transform.rotation_degrees();
    let (start_x, start_y) = transform.apply(cx - (rx / scale_x), cy);
    let (end_x, end_y) = transform.apply(cx + (rx / scale_x), cy);
    let d = format!(
        "M{} {} A{} {} {} 0 1 {} {} A{} {} {} 0 1 {} {} Z",
        fmt_f64(start_x),
        fmt_f64(start_y),
        fmt_f64(rx),
        fmt_f64(ry),
        fmt_f64(rotation),
        fmt_f64(end_x),
        fmt_f64(end_y),
        fmt_f64(rx),
        fmt_f64(ry),
        fmt_f64(rotation),
        fmt_f64(start_x),
        fmt_f64(start_y),
    );

    writer.start_element("path");
    for (key, value) in attrs {
        if matches!(key.as_str(), "cx" | "cy" | "rx" | "ry" | "r") {
            continue;
        }
        writer.write_attribute(key, value);
    }
    writer.write_attribute("d", &d);
    writer.end_element();
    Ok(())
}

fn required_svg_number(attrs: &BTreeMap<String, String>, key: &str) -> Result<f64> {
    attrs
        .get(key)
        .with_context(|| format!("missing {:?} SVG attribute", key))?
        .parse::<f64>()
        .with_context(|| format!("invalid {:?} SVG number", key))
}

fn normalize_color(value: &str) -> String {
    let mut value = value.trim().to_ascii_lowercase();
    if value == "none" || value.starts_with("url(") || value.starts_with("var(") {
        return value;
    }
    if value.starts_with('#') && value.len() == 4 {
        let mut expanded = String::from("#");
        for ch in value.chars().skip(1) {
            expanded.push(ch);
            expanded.push(ch);
        }
        value = expanded;
    }
    value
}

fn rewrite_paint(
    attrs: &mut BTreeMap<String, String>,
    icon_name: &str,
    paint_attr: &str,
    paint_value: &str,
) -> Result<()> {
    let color = normalize_color(paint_value);
    if color == "none" || color.starts_with("url(") || color.starts_with("var(") {
        attrs.insert(paint_attr.to_string(), color);
        return Ok(());
    }

    let palette_name = match color.as_str() {
        // primary colors: blue, red, green, yellow
        "#92d3f5" => "blue",
        "#61b2e4" => "blue-shade",
        //"#1e50a0" => "blue-deep", // XXX: does this case exist?
        "#ea5a47" => "red",
        "#d22f27" => "red-shade",
        //"#781e32" => "red-deep", // XXX: does this case exist?
        "#b1cc33" => "green",
        "#5c9e31" => "green-shade",
        "#fcea2b" => "yellow",
        "#f1b31c" => "yellow-shade",
        // gray scale:
        "#ffffff" => "white",
        "#d0cfce" => "gray-light",
        "#9b9b9a" => "gray",
        "#3f3f3f" => "gray-dark",
        "#000000" => "ink",
        // auxiliary colors: pink, purple, orange, brown
        "#ffa7c0" => "pink",
        "#e67a94" => "pink-shade",
        "#b399c8" => "purple",
        "#8967aa" => "purple-shade",
        "#f4aa41" => "orange",
        "#e27022" => "orange-shade",
        "#a57939" => "brown",
        "#6a462f" => "brown-shade",
        other => bail!(
            "OpenMoji icon {:?} uses unsupported {} color {:?}",
            icon_name,
            paint_attr,
            other
        ),
    };

    append_class(attrs, &format!("dtn-palette-{paint_attr}-{palette_name}"));
    attrs.insert(
        paint_attr.to_string(),
        format!("var(--dtn-sprite-{palette_name})"),
    );
    Ok(())
}

fn append_class(attrs: &mut BTreeMap<String, String>, class_name: &str) {
    let existing = attrs.get("class").cloned().unwrap_or_default();
    let mut classes: Vec<String> = existing.split_whitespace().map(ToOwned::to_owned).collect();
    if !classes.iter().any(|existing| existing == class_name) {
        classes.push(class_name.to_string());
    }
    attrs.insert("class".to_string(), classes.join(" "));
}

fn write_iosevka_glyphs(paths: &AssetPaths) -> Result<()> {
    ensure_generated_dir(paths)?;

    let mut writer = new_xml_writer();
    writer.start_element("svg");
    writer.write_attribute("xmlns", SVG_NS);
    writer.start_element("defs");

    for glyph_spec in GLYPH_SPECS {
        let font_path = paths
            .local_iosevka_repo
            .join("dist/IosevkaCustom/TTF-Unhinted")
            .join(glyph_spec.source);
        let font_data = fs::read(&font_path)
            .with_context(|| format!("failed to read {}", font_path.display()))?;
        let face = Face::parse(&font_data, 0)
            .with_context(|| format!("failed to parse {}", font_path.display()))?;

        let mut max_advance = 0.0f32;
        let mut union_bounds: Option<GlyphBounds> = None;
        for ch in glyph_spec.chars.chars() {
            let glyph_id = face
                .glyph_index(ch)
                .with_context(|| format!("missing glyph {:?} in {}", ch, font_path.display()))?;
            let advance = face.glyph_hor_advance(glyph_id).with_context(|| {
                format!(
                    "missing advance for glyph {:?} in {}",
                    ch,
                    font_path.display()
                )
            })? as f32;
            max_advance = max_advance.max(advance);

            let mut sink = BoundsOnlyBuilder;
            if let Some(bounds) = face.outline_glyph(glyph_id, &mut sink) {
                union_bounds = Some(match union_bounds {
                    None => GlyphBounds::from(bounds),
                    Some(existing) => existing.union(GlyphBounds::from(bounds)),
                });
            }
        }

        let union_bounds = union_bounds.with_context(|| {
            format!("could not compute glyph bounds for {}", font_path.display())
        })?;
        let source_height = union_bounds.y_max - union_bounds.y_min;
        let vertical_padding = source_height * 0.1;
        let source_box_height = source_height + (2.0 * vertical_padding);
        let source_top = union_bounds.y_max + vertical_padding;
        let scale_x = glyph_spec.view_box.0 / max_advance;
        let scale_y = glyph_spec.view_box.1 / source_box_height;

        for ch in glyph_spec.chars.chars() {
            let glyph_id = face
                .glyph_index(ch)
                .with_context(|| format!("missing glyph {:?} in {}", ch, font_path.display()))?;
            let mut path_builder = SvgPathBuilder::new(scale_x, scale_y, source_top);
            if face.outline_glyph(glyph_id, &mut path_builder).is_none() || path_builder.is_empty()
            {
                continue;
            }

            writer.start_element("symbol");
            writer.write_attribute(
                "id",
                &format!("dtn-glyph-{}-{}", glyph_spec.id, glyph_symbol_name(ch)),
            );
            writer.write_attribute(
                "viewBox",
                &format!(
                    "0 0 {} {}",
                    fmt_f32(glyph_spec.view_box.0),
                    fmt_f32(glyph_spec.view_box.1)
                ),
            );

            writer.start_element("path");
            writer.write_attribute(
                "class",
                &format!("dtn-glyph-path dtn-glyph-path-{}", glyph_spec.id),
            );
            writer.write_attribute("fill", "currentColor");
            writer.write_attribute("d", &path_builder.finish());
            writer.end_element();

            writer.end_element();
        }
    }

    writer.end_element();
    writer.end_element();

    fs::write(&paths.iosevka_glyphs, writer.end_document())
        .with_context(|| format!("failed to write {}", paths.iosevka_glyphs.display()))?;
    println!("Wrote {}", paths.iosevka_glyphs.display());
    Ok(())
}

fn glyph_symbol_name(ch: char) -> String {
    match ch {
        '-' => "minus".to_string(),
        '×' => "times".to_string(),
        _ => ch.to_string(),
    }
}

fn write_combined_sprite(paths: &AssetPaths) -> Result<()> {
    ensure_generated_dir(paths)?;
    if !paths.iosevka_glyphs.is_file() {
        bail!(
            "Missing generated glyph fragment at {}. Run `cargo run -p xtask -- regen-fonts` first.",
            paths.iosevka_glyphs.display()
        );
    }
    if !paths.openmoji_symbols.is_file() {
        bail!(
            "Missing generated OpenMoji fragment at {}. Run `cargo run -p xtask -- regen-sprite` first.",
            paths.openmoji_symbols.display()
        );
    }

    let openmoji_fragment = fs::read_to_string(&paths.openmoji_symbols)
        .with_context(|| format!("failed to read {}", paths.openmoji_symbols.display()))?;
    let openmoji_doc = Document::parse(&openmoji_fragment)
        .with_context(|| format!("failed to parse {}", paths.openmoji_symbols.display()))?;
    let glyph_fragment = fs::read_to_string(&paths.iosevka_glyphs)
        .with_context(|| format!("failed to read {}", paths.iosevka_glyphs.display()))?;
    let glyph_doc = Document::parse(&glyph_fragment)
        .with_context(|| format!("failed to parse {}", paths.iosevka_glyphs.display()))?;

    let mut writer = new_xml_writer();
    writer.start_element("svg");
    writer.write_attribute("id", "dtn-sprite-sheet");
    writer.write_attribute("class", "dtn-sprite-sheet");
    writer.write_attribute("xmlns", SVG_NS);
    writer.write_attribute("aria-hidden", "true");
    writer.write_attribute("focusable", "false");
    writer.write_attribute("width", "0");
    writer.write_attribute("height", "0");
    writer.write_attribute(
        "style",
        "position:absolute;width:0;height:0;overflow:hidden",
    );
    writer.start_element("defs");
    writer.start_element("style");
    writer.write_text(OPENMOJI_PALETTE_STYLE);
    writer.end_element();

    copy_fragment_symbols(&mut writer, &openmoji_doc)?;
    copy_fragment_symbols(&mut writer, &glyph_doc)?;

    writer.end_element();
    writer.end_element();

    fs::write(&paths.sprite, writer.end_document())
        .with_context(|| format!("failed to write {}", paths.sprite.display()))?;
    println!("Wrote {}", paths.sprite.display());
    Ok(())
}

fn copy_fragment_symbols(writer: &mut XmlWriter, document: &Document<'_>) -> Result<()> {
    let root = document.root_element();
    let defs = root
        .children()
        .find(|child| child.is_element() && child.tag_name().name() == "defs")
        .context("expected <defs> in generated fragment")?;

    for child in defs.children().filter(|child| child.is_element()) {
        if child.tag_name().name() == "symbol" {
            copy_svg_element(writer, child);
        }
    }

    Ok(())
}

fn copy_svg_element(writer: &mut XmlWriter, node: Node<'_, '_>) {
    writer.start_element(node.tag_name().name());
    let mut attrs: BTreeMap<&str, &str> = BTreeMap::new();
    for attribute in node
        .attributes()
        .filter(|attribute| attribute.namespace().is_none())
    {
        attrs.insert(attribute.name(), attribute.value());
    }
    for (key, value) in attrs {
        writer.write_attribute(key, value);
    }
    for child in node.children() {
        if child.is_element() {
            copy_svg_element(writer, child);
        } else if child.is_text() {
            if let Some(text) = child.text() {
                if !text.is_empty() {
                    writer.write_text(text);
                }
            }
        }
    }
    writer.end_element();
}

fn ensure_generated_dir(paths: &AssetPaths) -> Result<()> {
    fs::create_dir_all(&paths.generated_dir)
        .with_context(|| format!("failed to create {}", paths.generated_dir.display()))
}

fn new_xml_writer() -> XmlWriter {
    let options = Options {
        indent: Indent::Spaces(2),
        ..Options::default()
    };
    XmlWriter::new(options)
}

fn run(command: &mut Command) -> Result<()> {
    let status = run_allow_failure(command)?;
    if !status.success() {
        bail!("command {:?} exited with {}", command, status);
    }
    Ok(())
}

fn run_allow_failure(command: &mut Command) -> Result<std::process::ExitStatus> {
    let status = command
        .status()
        .with_context(|| format!("failed to spawn {:?}", command))?;
    Ok(status)
}

fn run_capture(command: &mut Command) -> Result<String> {
    let output = run_capture_allow_failure(command)?;
    if !output.status.success() {
        bail!(
            "command {:?} exited with {}: {}",
            command,
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn run_capture_optional(command: &mut Command) -> Result<Option<String>> {
    let output = run_capture_allow_failure(command)?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
    ))
}

fn run_capture_allow_failure(command: &mut Command) -> Result<std::process::Output> {
    let output = command
        .output()
        .with_context(|| format!("failed to spawn {:?}", command))?;
    Ok(output)
}

fn normalize_git_remote_url(url: &str) -> String {
    let url = url.trim();

    if let Some(rest) = url.strip_prefix("git@github.com:") {
        return format!("https://github.com/{}", rest.trim_end_matches(".git"));
    }
    if let Some(rest) = url.strip_prefix("ssh://git@github.com/") {
        return format!("https://github.com/{}", rest.trim_end_matches(".git"));
    }
    if let Some(rest) = url.strip_prefix("https://github.com/") {
        return format!("https://github.com/{}", rest.trim_end_matches(".git"));
    }
    if let Some(rest) = url.strip_prefix("http://github.com/") {
        return format!("https://github.com/{}", rest.trim_end_matches(".git"));
    }

    url.trim_end_matches(".git").to_string()
}

#[derive(Clone, Copy, Debug)]
struct GlyphBounds {
    x_min: f32,
    y_min: f32,
    x_max: f32,
    y_max: f32,
}

impl GlyphBounds {
    fn from(bounds: ttf_parser::Rect) -> Self {
        Self {
            x_min: bounds.x_min as f32,
            y_min: bounds.y_min as f32,
            x_max: bounds.x_max as f32,
            y_max: bounds.y_max as f32,
        }
    }

    fn union(self, other: Self) -> Self {
        Self {
            x_min: self.x_min.min(other.x_min),
            y_min: self.y_min.min(other.y_min),
            x_max: self.x_max.max(other.x_max),
            y_max: self.y_max.max(other.y_max),
        }
    }
}

struct BoundsOnlyBuilder;

impl OutlineBuilder for BoundsOnlyBuilder {
    fn move_to(&mut self, _x: f32, _y: f32) {}
    fn line_to(&mut self, _x: f32, _y: f32) {}
    fn quad_to(&mut self, _x1: f32, _y1: f32, _x: f32, _y: f32) {}
    fn curve_to(&mut self, _x1: f32, _y1: f32, _x2: f32, _y2: f32, _x: f32, _y: f32) {}
    fn close(&mut self) {}
}

struct SvgPathBuilder {
    data: String,
    scale_x: f32,
    scale_y: f32,
    source_top: f32,
}

impl SvgPathBuilder {
    fn new(scale_x: f32, scale_y: f32, source_top: f32) -> Self {
        Self {
            data: String::new(),
            scale_x,
            scale_y,
            source_top,
        }
    }

    fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    fn finish(self) -> String {
        self.data
    }

    fn transform(&self, x: f32, y: f32) -> (f32, f32) {
        (x * self.scale_x, (self.source_top - y) * self.scale_y)
    }

    fn push_cmd(&mut self, cmd: char, values: &[(f32, f32)]) {
        if !self.data.is_empty() {
            self.data.push(' ');
        }
        self.data.push(cmd);
        let mut first = true;
        for (x, y) in values {
            if !first {
                self.data.push(' ');
            }
            first = false;
            let _ = write!(&mut self.data, "{} {}", fmt_f32(*x), fmt_f32(*y));
        }
    }
}

impl OutlineBuilder for SvgPathBuilder {
    fn move_to(&mut self, x: f32, y: f32) {
        let point = self.transform(x, y);
        self.push_cmd('M', &[point]);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let point = self.transform(x, y);
        self.push_cmd('L', &[point]);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let control = self.transform(x1, y1);
        let point = self.transform(x, y);
        self.push_cmd('Q', &[control, point]);
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let control1 = self.transform(x1, y1);
        let control2 = self.transform(x2, y2);
        let point = self.transform(x, y);
        self.push_cmd('C', &[control1, control2, point]);
    }

    fn close(&mut self) {
        if !self.data.is_empty() {
            self.data.push(' ');
        }
        self.data.push('Z');
    }
}

fn fmt_f32(value: f32) -> String {
    if (value - value.round()).abs() < 0.0001 {
        return format!("{}", value.round() as i32);
    }
    let mut formatted = format!("{value:.4}");
    while formatted.contains('.') && formatted.ends_with('0') {
        formatted.pop();
    }
    if formatted.ends_with('.') {
        formatted.pop();
    }
    formatted
}

fn fmt_f64(value: f64) -> String {
    if (value - value.round()).abs() < 0.0001 {
        return format!("{}", value.round() as i32);
    }
    let mut formatted = format!("{value:.4}");
    while formatted.contains('.') && formatted.ends_with('0') {
        formatted.pop();
    }
    if formatted.ends_with('.') {
        formatted.pop();
    }
    formatted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imported_svg_normalizer_removes_identity_translate_transform() {
        let input = r##"
            <svg id="emoji" viewBox="0 0 72 72" xmlns="http://www.w3.org/2000/svg">
              <g id="line">
                <path transform="translate(0 0)" d="M1 2L3 4" stroke="#000000" fill="none"/>
              </g>
            </svg>
        "##;

        let normalized = normalize_openmoji_imported_svg(input).expect("svg should normalize");
        let document = Document::parse(&normalized).expect("normalized svg should parse");
        let path = document
            .descendants()
            .find(|node| node.has_tag_name("path"))
            .expect("path should remain present");

        assert!(!normalized.contains("transform="));
        assert_eq!(path.attribute("d"), Some("M1 2L3 4"));
        assert_eq!(path.attribute("fill"), Some("none"));
        assert_eq!(path.attribute("stroke"), Some("#000000"));
    }

    #[test]
    fn imported_svg_normalizer_flattens_transformed_ellipse_to_path() {
        let input = r##"
            <svg id="emoji" viewBox="0 0 72 72" xmlns="http://www.w3.org/2000/svg">
              <g id="color">
                <ellipse
                  cx="29.5854"
                  cy="24.8305"
                  rx="11.1656"
                  ry="11.1657"
                  transform="matrix(0.8006 -0.5992 0.5992 0.8006 -8.979 22.6777)"
                  fill="#FFFFFF"
                  stroke="none"
                />
              </g>
            </svg>
        "##;

        let normalized = normalize_openmoji_imported_svg(input).expect("svg should normalize");
        let document = Document::parse(&normalized).expect("normalized svg should parse");
        let path = document
            .descendants()
            .find(|node| node.has_tag_name("path"))
            .expect("ellipse should become a path");

        assert!(!normalized.contains("transform="));
        assert_eq!(path.attribute("fill"), Some("#FFFFFF"));
        assert_eq!(path.attribute("stroke"), Some("none"));
        assert!(
            path.attribute("d")
                .is_some_and(|value| value.contains('A') && value.ends_with('Z'))
        );
    }
}
