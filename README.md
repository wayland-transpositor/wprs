# wprs

Like [xpra](https://en.wikipedia.org/wiki/Xpra), but for Wayland, and written in
Rust.

wprs implements rootless remote desktop access for remote Wayland (and X11, via
XWayland) applications.

## Building

wprs is currently only available on x86-64 with AVX2. Support for [ARM](https://github.com/wayland-transpositor/wprs/issues/31) and support for more fast compression implementations welcome.

Currently building wprs without AVX2 will lead to build failures.

### Platform Support

- `wprsc` (client) is intended to be cross-platform and should build on Linux/macOS/Windows.
- `wprsd` has multiple backends:
  - Wayland compositor backend (Linux/Wayland): requires the `server` feature (Smithay) and is not supported on Apple platforms.
  - Fullscreen capture backends (macOS/Windows): use OS screen capture + input injection APIs (macOS requires Screen Recording + Accessibility permissions).

In practice:

- For macOS/Windows development, build `wprsc` only.
- For Linux deployment, build both `wprsc` and `wprsd`.

### Source

```bash
cargo build --profile=release-lto  # or release, but debug is unusably slow
```

### Cross Compilation (cross)

This repo includes a `Cross.toml` with common Linux targets and the system
dependencies needed to build the Wayland components.

Examples:

```bash
# Linux x86_64
cross build --target x86_64-unknown-linux-gnu --profile=release-lto --bin wprsc

# Linux aarch64
cross build --target aarch64-unknown-linux-gnu --profile=release-lto --bin wprsc
```

On non-Linux hosts (for example macOS), only the client is expected to build.
By default, `wprsc` builds with the cross-platform client backend:

```bash
cargo build --bin wprsc
```

You can also run a self-contained server demo that speaks the protocol and streams a
synthetic surface (no Wayland compositor / Smithay required):

```bash
cargo run --example wprsd_demo
```

Then connect to it with the cross-platform client backend:

```bash
cargo run --bin wprsc -- --socket /path/printed/by/demo.sock
```

The following dependencies are required for `wprsc`, `wprsd`, `xwayland-xdg-shell`:

* libxkbcommon (-dev on debian)
* libwayland (-dev on debian)

The launcher (`wprs`) requires:

* python3
* psutil (python3-psutil on debian)
* ssh client

## Packaging

This repo includes packaging templates (Arch/Nix/Homebrew) and a local packaging script.

### Local Packaging Script

For local, repeatable packaging into per-target artifacts (archives + optional distro-native packages), use:

```bash
./scripts/package.sh
```

For `deb`/`rpm` outputs, the script uses a small Docker/Podman container with packaging tools installed from the distro repos.

Outputs are written under `dist/`.

### Arch-Linux (AUR)

wprs is available from the [Arch User Repository](https://aur.archlinux.org/packages/wprs-git)
as `wprs-git`

## Usage

On the remote host, put the `wprsd.service` file into place:

```bash
mkdir -p ~/.config/systemd/user
cp package/wprsd.service ~/.config/systemd/user
```

and enable wprsd:

```bash
loginctl enable-linger
systemctl --user enable wprsd.service
systemctl --user start wprsd.service
```

On the local host:
```bash
# starts application on the remote host (starts ssh connection, forwards sockets, starts wprsc, runs application)
wprs <remote_host> run <application>

# stops local wprs connections, leaving remote session running (tear down ssh connection and forwarded sockets, stops wprsc)
wprs <remote_host> detach

# attaches to remote wprs session (starts ssh connection, forwards sockets, starts wprsc)
wprs <remote_host> attach
```

## System Tuning

Increasing linux's socket buffer limits as described in
<https://wiki.archlinux.org/title/sysctl#Increase_the_memory_dedicated_to_the_network_interfaces>
will result in improved performance.

TODO: test ssh socket forwarding performance with different values of
wmem_default. wprs uses setsockopt to increase its buffer size, but it doesn't
seem that ssh does.

## Configuration Files

You can create configuration files for `wprsc` and `wprsd` instead of passing additional
arguments to `wprs`. To see what options are available, run `wprsc --help` and
`wprsd --help`.

To generate the default configs, run:
```bash
# on your local machine
wprsc --print-default-config-and-exit=true > ~/.config/wprs/wprsc.ron
```
and
```bash
# on your remote machine
wprsd --print-default-config-and-exit=true > ~/.config/wprs/wprsd.ron
```

Then update the `wprsc.ron` and `wprsd.ron` files with your desired settings.

### Running `wprsc` Without Wayland (Experimental)

`wprsc` is normally a Wayland client and requires a local Wayland compositor.
For development and experimentation on non-Wayland desktops (for example macOS
or Windows), `wprsc` also supports a cross-platform backend using `winit` +
`pixels`.

When no Wayland compositor is detected, `wprsc` will automatically fall back to
this backend (it is enabled by default). You can override the selection with
`--backend auto|wayland|winit-pixels`.

```bash
cargo run --profile dev --bin wprsc
```

Keyboard behavior is configurable:

* `--keyboard-mode=keymap` (default): try to send an explicit XKB keymap to the
  server. By default `wprsc` will try to generate one at runtime using external
  tools (`setxkbmap` + `xkbcomp`). You can override by providing an explicit
  file via `--xkb-keymap-file=/path/to/keymap`.
* `--keyboard-mode=evdev`: send Linux evdev keycodes without sending a keymap.

Current limitations of the `winit` + `pixels` backend:

* Only `xdg-toplevel` surfaces are displayed.
* Input forwarding is best-effort (pointer and basic keyboard).
  Keyboard events are translated to Linux evdev keycodes and may be incomplete
  on non-Linux hosts.
* Popups/subsurfaces are not fully supported.
* Pinch/rotation gestures are not forwarded yet.

## Current Limitations

Currently only the the Core and XDG shell protocols are implemented. In
particular, hardware rendering/dmabuf support is not yet implemented.

* Touch event support is not yet implemented.
* Drag-and-drop may be wonky in some cases.
* XWayland drag-and-drop is not (yet?) implemented.
* webauthn security keys don't yet work in browsers

Generally, wprs will aim to support as many protocols as feasible, it's a
question of time and prioritization.

## Architecture

On the remote (server) side, `wprsd` implements a wayland compositor using
[Smithay](https://github.com/Smithay/smithay). Instead of compositing and
rendering though, wprsd serializes the state of the wayland session and sends it
to the connected wprsc client using a custom protocol.

On the local (client) side, `wprsc` implements a wayland client (using the
[Smithay Client Toolkit](https://github.com/Smithay/client-toolkit) that creates
local wayland objects that correspond to remote wayland objects. For example, if
a remote application running against wprsd creates a surface and an
xdg-toplevel, wprsc will create a surface with the same contents, an
xdg-toplevel with the same metadata, etc.. From the local compositor's point of
view, wprsc is just a normal application with a bunch of windows. Input and
other events from the local compositor that wprsc are serialized and sent to
wprsd, which forwards them to the appropriate application (the owner of the
surface which the wprsc surface which received the events corresponds to).

wprs supports session resumption across temporary disconnects. By default,
`wprsc` will automatically reconnect to `wprsd` (disable with
`wprsc --no-auto-reconnect`). The wayland protocol is not natively resumable in this way
because it relies on shared state between the compositor and client
applications. By implementing a wayland compositor locally relative to the
application, wprsd stores all state necessary for wayland applications and is
also able to store sufficient state (e.g., the buffer contents for each surface
as of the last commit) for a newly-connected wprsc to correctly set up all
necessary wayland objects. wprsc is stateless, but wprsd is not, so a wprsd
restart will still terminate all wayland applications running against it, like
with any other wayland compositor.

Communication between wprsd and wprsc happens over unix domain sockets; wprsd
creates a socket and wprsc connects to it. The default mode of operation is to,
on the client side, use ssh to forward a local socket to the remote wprsd
socket, but a different transport could be used with, for example, socat or a
custom proxy application. A launcher script (`wprs`) is provided which sets up
the ssh socket forwarding.

### Protocol

The custom protocol used to serialize and transmit wayland state between wprsc
and wprsd is a simplified version of the wayland protocol. Wayland objects are
represented as rust types and serialized using
[rkyv](https://github.com/rkyv/rkyv). Unlike the wayland protocol, the wprs
protocol tries to be idempotent when possible. For example, instead of the
repeated back-and-forth involved in created a surface, creating an xdg-surface,
creating an xdg-toplevel, waiting for it to be configured, creating a buffer,
attaching the buffer, and comitting it, wprsd will send a single commit message
to wprsc with the complete state of the surface (surface's attached buffer
contents (if any), its role (if any) and any associated metadata, etc.) and
wprsc will execute the appropriate dance with the local compositor.

Frame callbacks are scheduled locally by wprsd at the configured framerate, they
are not forwarded from wprsc as that would introduce an unacceptable amount of
frame latency due to network round-trips. When no wprsc is connected, wprsd
pauses sending frame callbacks to wayland applications.

Buffer compression is handled using a custom multithreaded and SIMD-accelerated
lossless image compression algorithm:

1. Transpose the image from an [array of structures to a struct of
   arrays](https://en.wikipedia.org/wiki/AoS_and_SoA). This makes the subequent
   steps significantly faster by letting them be implemented with SIMD
   instructions and additionally improves the compression ratio because each
   color channel is more closely spatially correlated with itself than with the
   other
   channels.
2. Apply an adjacent (wrapping) difference to each color channel (differential
   pulse-code modulation). This improves the compression ratio by taking
   advantage of spatial correlation and transforms (for example) a solid-colored
   line into a single color byte and then a sequence of 0-bytes, or a gradient
   into a sequence of 1-bytes, etc.
3. Transform each color channel into a
   [YUV](https://en.wikipedia.org/wiki/Y%E2%80%B2UV)-like color space: `y := g,
   u := b - g, v := r - g, a := a`. This improves the compression ratio in a
   similar way as the previous step but by taking advantage of cross-color
   correlation.
4. Compress the data with zstd.

This algorithm was designed for reasonably good compression ratios while being
extremely fast: single-digit milliseconds per frame. Decompression is done by
inverting those steps.

This protocol is *not stable*: there is no guarantee that different versions of
wprsc and wprsd, or wprsc and wprsd built with different versions of
dependencies or even rustc will be compatible. This may change in the future,
but it will not happen soon.

### Comparison to Waypipe

[Waypipe](https://gitlab.freedesktop.org/mstoeckl/waypipe)'s model is analogous
to X forwarding, while wprs's model is analgous to Xpra. Waypipe ~transparently
forwards messages between the local compositor and the remote application, so
the client ends up being stateful and sessions can only be resumed through
network reconnections, not client restarts. There are tradeoffs to the two
approaches. Waypipe's approach is partially forward-compatible: it can support
new wayland protocols automatically, however those protocols may be broken if
they use shared resources in a way that waypipe doesn't know how to handle.
wprs, on the other hand, requires explicit implementation for every wayland
protocol.

### XWayland

XWayland support is implemented via the helper `xwayland-xdg-shell` by default.
The helper implements a Wayland compositor (but only for the protocol features used
by Xwayland) and client, just like wprsd and wprsc, but in a single binary (so
skipping the serialization/deserialization).

For deployments that prefer fewer processes, wprsd can also run the Xwayland
proxy inline (without spawning `xwayland-xdg-shell`) by setting
`xwayland_mode = "inline-proxy"`.

To run the helper without letting wprsd spawn it (e.g. if you want separate
process supervision), set `xwayland_mode = "external"` and start
`xwayland-xdg-shell` yourself with `WAYLAND_DISPLAY` pointing at wprsd.

### HiDPI

wprs tracks scale using Wayland-style semantics:

- `SurfaceState.buffer_scale` is the number of buffer pixels per logical point.
  For example, a Retina capture typically uses `buffer_scale = 2`.
- `wprsd` sends a `DisplayConfig` message on connect (currently used by capture
  backends to advertise a best-effort DPI/scale).

For the macOS fullscreen capture backend, wprsd detects the main display scale
factor and reports it via both `DisplayConfig.scale_factor` and
`SurfaceState.buffer_scale`.

Override knobs:

- Server DPI (generic): set `display_dpi = Some(110)` in the `wprsd` config (or
  pass `--display-dpi 110`). This only affects backends that use it.
- Client-side scaling (generic): set `ui_scale_factor = 1.25` in the `wprsc`
  config (or pass `--ui-scale-factor 1.25`) to scale window sizes for
  cross-platform clients.

The helper binary model is the same as
[xwayland-proxy-virtwl](https://github.com/talex5/wayland-proxy-virtwl#xwayland-support),
which is itself inspired by
[sommelier](https://chromium.googlesource.com/chromiumos/platform2/+/main/vm_tools/sommelier/).
xwayland-xdg-shell was primarily written (instead of just using
xwayland-proxy-virtwl) so as to share a common design/codebase with wprs and to
make use of common wayland development in the form of Smithay and its wayland
crates. Additionally, xwayland-xdg-shell is more narrowly focused and its sole
purpose is xwayland support, not virtio-gpu or virtwl.

Like xwayland-proxy-virtwl, xwayland-xdg-shell can be used to implement external
xwayland support for any wayland compositor instead of re-implementing it inside
the compositor. Aside from eliminating the need to implement xwayland support in
every compositor, this approach has been reported to result in better xwayland
scaling than native xwayland support in some compositor, and it allows xwayland
applications to be treated more like regular wayland applications instead of
getting special access.

### Security

wprsd is a wayland compositor, so it has access to all surfaces displayed by
applications running against it and it can inject input into them. Any process
which implements the wprs protocol and connects to the wprs socket will have the
same access. For that reason, the wprs socket is created in a directory which
only the user has access to ($XDG_RUNTIME_DIR) and the socket itself is only
readable/writable by the user. Malicious applications running as the same user
as wprsd can still access this socket, but at that point you have bigger
problems.

wprs does not do any auth of its own, it relies entirely on whatever transport
is being used (ssh, in the default case).

## Thanks

Huge thanks to the following excellent projects for making this project
significantly easier than it otherwise would have been:

* [Smithay](https://github.com/Smithay)
* [rkyv](https://github.com/rkyv/rkyv)
* [tracing](https://github.com/tokio-rs/tracing)
* [Tracy](https://github.com/wolfpld/tracy)

Thanks to [Waypipe](https://gitlab.freedesktop.org/mstoeckl/waypipe) and
[xwayland-proxy-virtwl](https://github.com/talex5/wayland-proxy-virtwl#xwayland-support)
for paving the way in this problem space.
