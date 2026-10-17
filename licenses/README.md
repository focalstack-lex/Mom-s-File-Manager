# License texts Tote ships for packages that carry none

`scripts/third-party-notices.mjs` prints every shipped package's own license file into `THIRD-PARTY-NOTICES.txt`. A few packages publish no license file in their crate, so their texts live here, one folder per package name (`licenses/<name>/LICENSE*`), and the generator uses a folder only when the package has no file of its own. It fails, and writes nothing, when a shipped package has a text in neither place (audit T-39, 2026-10-16), so a new dependency without one is found at release time, not by a reader of the notices.

`webview2-loader/` is not a package: `webview2-com-sys` links Microsoft's `WebView2LoaderStatic.lib` from the WebView2 SDK into `tote.exe`. The generator lists it under its own heading and fails when `webview2-com-sys` changes version, because a new version may link a different SDK; then check the SDK version again (`WEBVIEW2_VERSION` in `crates/update-bindings/src/main.rs` of github.com/wravery/webview2-rs at the commit in the crate's `.cargo_vcs_info.json`), compare its license, and update `LINKED` in the generator.

Where each text came from (fetched 2026-10-17, at the commit the crate version was packaged from, as its `.cargo_vcs_info.json` records):

| Folder | Package and version | Source |
| --- | --- | --- |
| `adobe-cmap-parser` | adobe-cmap-parser 0.4.1 (MIT) | No license file in the crate or in github.com/jrmuizel/adobe-cmap-parser at `ac107d55`; the MIT terms its Cargo.toml declares, with the author it names. |
| `alloc-stdlib` | alloc-stdlib 0.3.0 (BSD-3-Clause) | `LICENSE` at the root of github.com/dropbox/rust-alloc-no-stdlib at `0a81fd69` (the crate is its `alloc-stdlib` folder). |
| `clipboard-win` | clipboard-win 5.4.1 (BSL-1.0) | `LICENSE` of github.com/DoumanAsh/clipboard-win at `3b27cf2b`. |
| `defmt-parser` | defmt-parser 1.0.0 (MIT OR Apache-2.0) | `LICENSE-MIT` and `LICENSE-APACHE` at the root of github.com/knurling-rs/defmt at `4a8cdb44` (the crate is its `parser` folder). |
| `pdf-extract` | pdf-extract 0.12.1 (MIT) | No license file in the crate or in github.com/jrmuizel/pdf-extract at `47b792ba`; the MIT terms its Cargo.toml declares, with the author it names. |
| `selectors` | selectors 0.38.0 (MPL-2.0) | No license file in the crate or at the root of github.com/servo/stylo at `572ecba2`; its source files point to https://mozilla.org/MPL/2.0/, and this is that text (https://www.mozilla.org/media/MPL/2.0/index.txt). |
| `type1-encoding-parser` | type1-encoding-parser 0.1.1 (MIT) | No license file in the crate or in github.com/jrmuizel/type1-encoding-parser at `55d193e2`; the MIT terms its Cargo.toml declares, with the author it names. |
| `webview2-com` | webview2-com 0.39.1 (MIT) | `LICENSE` at the root of github.com/wravery/webview2-rs at `edc2caf8`. |
| `webview2-com-macros` | webview2-com-macros 0.8.1 (MIT) | `LICENSE` at the root of github.com/wravery/webview2-rs at `dffa41a8`. |
| `webview2-com-sys` | webview2-com-sys 0.39.1 (MIT) | `LICENSE` at the root of github.com/wravery/webview2-rs at `edc2caf8`. |
| `webview2-loader` | Microsoft WebView2 SDK 1.0.3800.47, `WebView2LoaderStatic.lib` | The package's `LICENSE.txt` as nuget.org shows it (https://www.nuget.org/packages/Microsoft.Web.WebView2/1.0.3800.47/License). |

The upstream texts are third-party texts, copied as published and never reworded. The three MIT files for Jeff Muizelaar's crates have no upstream file to copy, so each starts with a short note saying so before the MIT terms. When a package starts shipping its own file, its own file wins and the folder here can go.
