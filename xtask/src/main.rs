use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use roxmltree::{Document, Node};
use ttf_parser::{Face, OutlineBuilder};
use xmlwriter::{Indent, Options, XmlWriter};

const SVG_NS: &str = "http://www.w3.org/2000/svg";
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

    match cli.command {
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

fn sync_openmoji(paths: &AssetPaths, openmoji_dir: PathBuf) -> Result<()> {
    if !openmoji_dir.is_dir() {
        bail!(
            "OpenMoji directory does not exist: {}",
            openmoji_dir.display()
        );
    }

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
        fs::copy(&source_path, &destination).with_context(|| {
            format!(
                "failed to copy {} to {}",
                source_path.display(),
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

fn regen_sprite(paths: &AssetPaths) -> Result<()> {
    let manifest = load_openmoji_manifest(paths)?;
    write_openmoji_symbols(paths, &manifest)?;
    write_combined_sprite(paths)
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
