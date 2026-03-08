<div align="center">
  <h1><a href="https://sugoijan.dev/detonito/"<code>detonito</code></a></h1>
  <strong>Just a silly minesweeper clone.</strong>
</div>

## Build it yourself

For normal development builds you'll need Rust, trunk, Sass and just.

Use `just web` for local testing. For actual builds see [the CI script](.github/workflows/pages.yml).

## Asset maintenance

Normal builds use the committed source assets under [`web/assets`](web/assets) plus the committed generated sprite files under [`web/generated`](web/generated). Git submodules are not required for normal builds or CI.

Asset maintenance lives in the workspace `xtask` crate and the `just` recipes are thin wrappers around it.

To sync the curated upstream OpenMoji SVGs from the default `../openmoji` checkout and rebuild the combined sprite:

```sh
just sync-openmoji
```

To override the OpenMoji checkout path explicitly:

```sh
just sync-openmoji OPENMOJI_DIR=/path/to/openmoji
```

To rebuild the combined sprite from the committed OpenMoji sources and the committed Iosevka glyph fragment:

```sh
just regen-sprite
```

To regenerate the committed Iosevka SVG glyph fragment, keep a local checkout at `../Iosevka` and install Node.js:

```sh
just regen-fonts
```

To refresh both asset sets together:

```sh
just regen-assets
```

Or with an explicit OpenMoji checkout path:

```sh
just regen-assets OPENMOJI_DIR=/path/to/openmoji
```

## License

Licensed under either of

* Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
* MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.
