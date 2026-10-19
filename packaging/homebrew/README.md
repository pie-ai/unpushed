# Homebrew packaging

`unpushed.rb.in` is the formula template. The release workflow substitutes
`@VERSION@`, `@ARM_SHA@` and `@X86_SHA@` and pushes the result to
`pie-ai/homebrew-unpushed` as `Formula/unpushed.rb`.

The formula installs a prebuilt binary rather than building from source: the
macOS binaries are produced by the release workflow anyway, `brew install`
finishes immediately, and users do not need Rust 1.88 on their machine.
