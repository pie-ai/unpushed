# Third-party licenses

`unpushed` links against the following components. Nothing here is copyleft for
the resulting binary: every dependency is permissively licensed, and libgit2 —
the one GPL component — carries a linking exception that explicitly allows it.

## libgit2

libgit2 is compiled into the binary and linked statically (the
`vendored-libgit2` feature of the `git2` crate). It is licensed GPLv2 **with a
linking exception**:

> In addition to the permissions in the GNU General Public License, the authors
> give you unlimited permission to link the compiled version of this library
> into combinations with other programs, and to distribute those combinations
> without any restriction coming from the use of this file. (The General Public
> License restrictions do apply in other respects; for example, they cover
> modification of the file, and distribution when not linked into a combined
> executable.)

This exception is what permits distributing `unpushed` under MIT or Apache-2.0
while carrying libgit2 inside the executable. The full text ships with the
libgit2 sources: https://github.com/libgit2/libgit2/blob/main/COPYING

## Rust dependencies

Licenses as declared by each crate:

| Crate | Version | License |
|---|---|---|
| allocator-api2 | 0.2.21 | MIT OR Apache-2.0 |
| anstream | 1.0.0 | MIT OR Apache-2.0 |
| anstyle | 1.0.14 | MIT OR Apache-2.0 |
| anstyle-parse | 1.0.0 | MIT OR Apache-2.0 |
| anstyle-query | 1.1.5 | MIT OR Apache-2.0 |
| bitflags | 2.13.2 | MIT OR Apache-2.0 |
| castaway | 0.2.4 | MIT |
| cfg-if | 1.0.5 | MIT OR Apache-2.0 |
| clap | 4.6.7 | MIT OR Apache-2.0 |
| clap_builder | 4.6.7 | MIT OR Apache-2.0 |
| clap_derive | 4.6.7 | MIT OR Apache-2.0 |
| clap_lex | 1.1.1 | MIT OR Apache-2.0 |
| colorchoice | 1.0.5 | MIT OR Apache-2.0 |
| compact_str | 0.9.1 | MIT |
| convert_case | 0.10.0 | MIT |
| critical-section | 1.2.0 | MIT OR Apache-2.0 |
| crossterm | 0.29.0 | MIT |
| darling | 0.24.1 | MIT |
| darling_core | 0.24.1 | MIT |
| darling_macro | 0.24.1 | MIT |
| deranged | 0.5.9 | MIT OR Apache-2.0 |
| derive_more | 2.1.1 | MIT |
| derive_more-impl | 2.1.1 | MIT |
| document-features | 0.2.12 | MIT OR Apache-2.0 |
| either | 1.19.0 | MIT OR Apache-2.0 |
| equivalent | 1.0.3 | Apache-2.0 OR MIT |
| errno | 0.3.14 | MIT OR Apache-2.0 |
| foldhash | 0.2.0 | Zlib |
| git2 | 0.21.0 | MIT OR Apache-2.0 |
| hashbrown | 0.17.1 | MIT OR Apache-2.0 |
| heck | 0.5.0 | MIT OR Apache-2.0 |
| ident_case | 1.0.1 | MIT/Apache-2.0 |
| indoc | 2.0.8 | MIT OR Apache-2.0 |
| instability | 0.3.14 | MIT |
| is_terminal_polyfill | 1.70.2 | MIT OR Apache-2.0 |
| itertools | 0.14.0 | MIT OR Apache-2.0 |
| itoa | 1.0.18 | MIT OR Apache-2.0 |
| kasuari | 0.4.13 | MIT OR Apache-2.0 |
| libc | 0.2.191 | MIT OR Apache-2.0 |
| libgit2-sys | 0.18.8+1.9.7 | MIT OR Apache-2.0 |
| libz-sys | 1.1.30 | MIT OR Apache-2.0 |
| line-clipping | 0.3.8 | MIT OR Apache-2.0 |
| litrs | 1.0.1 | MIT OR Apache-2.0 |
| lock_api | 0.4.14 | MIT OR Apache-2.0 |
| log | 0.4.34 | MIT OR Apache-2.0 |
| lru | 0.18.5 | MIT |
| mio | 1.2.4 | MIT |
| num_threads | 0.1.8 | MIT OR Apache-2.0 |
| num-conv | 0.2.2 | MIT OR Apache-2.0 |
| parking_lot | 0.12.5 | MIT OR Apache-2.0 |
| parking_lot_core | 0.9.12 | MIT OR Apache-2.0 |
| powerfmt | 0.2.1 | MIT OR Apache-2.0 |
| proc-macro2 | 1.0.107 | MIT OR Apache-2.0 |
| quote | 1.0.47 | MIT OR Apache-2.0 |
| ratatui | 0.30.2 | MIT |
| ratatui-core | 0.1.2 | MIT |
| ratatui-crossterm | 0.1.2 | MIT |
| ratatui-macros | 0.7.2 | MIT |
| ratatui-widgets | 0.3.2 | MIT |
| rustix | 1.1.5 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| rustversion | 1.0.23 | MIT OR Apache-2.0 |
| ryu | 1.0.23 | Apache-2.0 OR BSL-1.0 |
| scopeguard | 1.2.0 | MIT OR Apache-2.0 |
| signal-hook | 0.3.18 | Apache-2.0/MIT |
| signal-hook-mio | 0.2.5 | MIT OR Apache-2.0 |
| signal-hook-registry | 1.4.8 | MIT OR Apache-2.0 |
| smallvec | 1.16.3 | MIT OR Apache-2.0 |
| static_assertions | 1.1.0 | MIT OR Apache-2.0 |
| strsim | 0.11.1 | MIT |
| strum | 0.28.0 | MIT |
| strum_macros | 0.28.0 | MIT |
| syn | 2.0.119 | MIT OR Apache-2.0 |
| syn | 3.0.9 | MIT OR Apache-2.0 |
| thiserror | 2.0.21 | MIT OR Apache-2.0 |
| thiserror-impl | 2.0.21 | MIT OR Apache-2.0 |
| time | 0.3.55 | MIT OR Apache-2.0 |
| time-core | 0.1.9 | MIT OR Apache-2.0 |
| unicode-ident | 1.0.27 | (MIT OR Apache-2.0) AND Unicode-3.0 |
| unicode-segmentation | 1.13.3 | MIT OR Apache-2.0 |
| unicode-truncate | 2.0.1 | MIT OR Apache-2.0 |
| unicode-width | 0.2.2 | MIT OR Apache-2.0 |
| utf8parse | 0.2.2 | Apache-2.0 OR MIT |

## Summary

- 54 crate(s): `MIT OR Apache-2.0`
- 20 crate(s): `MIT`
- 2 crate(s): `Apache-2.0 OR MIT`
- 1 crate(s): `Zlib`
- 1 crate(s): `MIT/Apache-2.0`
- 1 crate(s): `Apache-2.0/MIT`
- 1 crate(s): `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT`
- 1 crate(s): `Apache-2.0 OR BSL-1.0`
- 1 crate(s): `(MIT OR Apache-2.0) AND Unicode-3.0`
