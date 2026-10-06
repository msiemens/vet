# Only needed to build the Tauri shell on Linux (`ui/src-tauri`): the GTK and
# webkit system libraries. The Rust workspace, the UI and the backend need
# nothing beyond rustc/cargo, node and pnpm. Rust, node and pnpm come from the
# user profile.
{ pkgs ? import <nixpkgs> { } }:

pkgs.mkShell {
  nativeBuildInputs = [ pkgs.pkg-config ];
  buildInputs = [
    pkgs.dbus
    pkgs.webkitgtk_4_1
    pkgs.gtk3
    pkgs.libsoup_3
    pkgs.openssl
    pkgs.librsvg
  ];
}
