# Windows

Nothing here is wired up yet — there is no Windows build. When there is, the icon is two
lines of work, and both of them are already prepared.

**The executable's own icon.** `assets/icon/supersilvia.ico` holds 16 through 256. Embedding
it is a few lines more in `build.rs`, which today only links Syphon on macOS:

```rust
// build.rs, inside main()
    #[cfg(windows)]
    winresource::WindowsResource::new()
        .set_icon("assets/icon/supersilvia.ico")
        .compile()
        .unwrap();
```

with `[target.'cfg(windows)'.build-dependencies] winresource = "0.1"` in `Cargo.toml`. The
window icon itself is already handled: `src/main.rs` bakes the 256 PNG in with
`ViewportBuilder::with_icon`, which is what the taskbar and Alt-Tab use at run time.

**The installer.** Whatever it ends up being — MSI, NSIS, or a zip — points at the same
`.ico`. Nothing else in this directory exists yet on purpose.
