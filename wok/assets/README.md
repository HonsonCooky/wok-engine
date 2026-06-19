# Bundled assets

## SymbolsNerdFontMono-Regular.ttf

The icons-only "Symbols Nerd Font Mono" from [Nerd Fonts](https://github.com/ryanoasis/nerd-fonts), the
`NerdFontsSymbolsOnly` archive of release **v3.4.0**. It carries only the aggregated icon glyphs (no Latin text), so
it is embedded as a fallback font behind egui's own UI text: icon codepoints render through it, everything else
through the default font. The editor uses the Material Design Icons range (`nf-md-*`); see `wok/src/icons.rs` for the
exact codepoints in use.

Bundled rather than fetched at build time, matching the repo's dependency discipline (vendored, version-pinned, no
network in the build). To update, re-download `NerdFontsSymbolsOnly.zip` from the desired release and replace the ttf
+ license here.

- Source: https://github.com/ryanoasis/nerd-fonts/releases/tag/v3.4.0 (`NerdFontsSymbolsOnly.zip`)
- sha256 (ttf): `f0f624d9b474bea1662cf7e862d44aebe1ae1f6c7f9cb7a0ca5d0e5ac9561c60`
- License: MIT (`SymbolsNerdFont-LICENSE.txt`), Copyright (c) 2014 Ryan L McIntyre.
