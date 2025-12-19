use std::sync::Arc;

use anyhow::ensure;

use crate::server::runtime::backend::BackendObservation;
use crate::server::runtime::backend::PollingBackend;
use crate::server::runtime::backend::SurfaceSnapshot;
use crate::prelude::*;
use crate::protocols::wprs::Capabilities;
use crate::protocols::wprs::ClientId;
use crate::protocols::wprs::Event;
use crate::protocols::wprs::wayland::Buffer;
use crate::protocols::wprs::wayland::BufferAssignment;
use crate::protocols::wprs::wayland::BufferData;
use crate::protocols::wprs::wayland::BufferFormat;
use crate::protocols::wprs::wayland::BufferMetadata;
use crate::protocols::wprs::wayland::Role;
use crate::protocols::wprs::wayland::SurfaceState;
use crate::protocols::wprs::wayland::WlSurfaceId;
use crate::protocols::wprs::xdg_shell::XdgToplevelId;
use crate::protocols::wprs::xdg_shell::XdgToplevelState;

#[cfg(unix)]
use std::ffi::c_void;
#[cfg(unix)]
use std::num::NonZeroUsize;
#[cfg(unix)]
use std::ptr::NonNull;

#[cfg(unix)]
use nix::sys::mman;
#[cfg(unix)]
use x11rb::connection::RequestConnection;
#[cfg(unix)]
use x11rb::connection::Connection;
#[cfg(unix)]
use x11rb::protocol::shm;
#[cfg(unix)]
use x11rb::protocol::shm::ConnectionExt as _;
#[cfg(unix)]
use x11rb::protocol::xproto;
#[cfg(unix)]
use x11rb::protocol::xproto::ConnectionExt as _;
#[cfg(unix)]
use x11rb::rust_connection::RustConnection;

#[derive(Debug, Clone, Copy)]
struct PixmapFormatInfo {
    bits_per_pixel: u8,
    bytes_per_pixel: usize,
    bytes_per_line: usize,
}

impl PixmapFormatInfo {
    fn for_depth(
        setup: &x11rb::protocol::xproto::Setup,
        depth: u8,
        width: u16,
    ) -> Result<Self> {
        let format = setup
            .pixmap_formats
            .iter()
            .find(|fmt| fmt.depth == depth)
            .ok_or_else(|| anyhow!("missing pixmap format for depth {depth}"))?;

        let bits_per_pixel = format.bits_per_pixel;
        let scanline_pad = format.scanline_pad;

        ensure!(bits_per_pixel % 8 == 0, "unsupported bits-per-pixel: {bits_per_pixel}");
        ensure!(scanline_pad != 0, "invalid scanline pad: {scanline_pad}");

        let bytes_per_pixel = (bits_per_pixel / 8) as usize;
        let bytes_per_line = compute_stride_bytes(width, bits_per_pixel, scanline_pad)
            .ok_or_else(|| anyhow!("invalid stride for width={width} bpp={bits_per_pixel}"))?;

        Ok(Self {
            bits_per_pixel,
            bytes_per_pixel,
            bytes_per_line,
        })
    }
}

#[cfg(unix)]
#[derive(Debug)]
struct ShmCapture {
    shmseg: shm::Seg,
    map: NonNull<c_void>,
    map_len: usize,
    bytes_per_line: usize,
    bytes_per_pixel: usize,
}

#[cfg(unix)]
impl ShmCapture {
    fn create(
        conn: &RustConnection,
        segment_size: usize,
        bytes_per_line: usize,
        bytes_per_pixel: usize,
    ) -> Result<Option<Self>> {
        if conn
            .extension_information(shm::X11_EXTENSION_NAME)
            .location(loc!())?
            .is_none()
        {
            return Ok(None);
        }

        let shm_version = conn.shm_query_version().location(loc!())?.reply().location(loc!())?;
        let (major, minor) = (shm_version.major_version, shm_version.minor_version);
        // We rely on FD passing via CreateSegment/AttachFd (SHM >= 1.2).
        if (major, minor) < (1, 2) {
            return Ok(None);
        }

        ensure!(segment_size != 0, "invalid SHM segment size: 0");
        ensure!(
            segment_size <= u32::MAX as usize,
            "SHM segment too large: {segment_size}"
        );

        let shmseg = conn.generate_id().location(loc!())?;
        let reply = conn
            .shm_create_segment(shmseg, segment_size as u32, false)
            .location(loc!())?
            .reply()
            .location(loc!())?;
        let shm_fd = reply.shm_fd;

        let map_len = NonZeroUsize::new(segment_size).ok_or_else(|| anyhow!("segment_size=0"))?;
        let map = unsafe {
            mman::mmap(
                None,
                map_len,
                mman::ProtFlags::PROT_READ | mman::ProtFlags::PROT_WRITE,
                mman::MapFlags::MAP_SHARED,
                &shm_fd,
                0,
            )
            .location(loc!())?
        };

        conn.shm_attach_fd(shmseg, shm_fd, false)
            .location(loc!())?
            .check()
            .location(loc!())?;

        Ok(Some(Self {
            shmseg,
            map,
            map_len: segment_size,
            bytes_per_line,
            bytes_per_pixel,
        }))
    }

    fn detach(&self, conn: &RustConnection) -> Result<()> {
        conn.shm_detach(self.shmseg).location(loc!())?.check().location(loc!())?;
        Ok(())
    }
}

#[cfg(unix)]
impl Drop for ShmCapture {
    fn drop(&mut self) {
        if let Some(len) = NonZeroUsize::new(self.map_len) {
            let _ = unsafe { mman::munmap(self.map, len.get()) };
        }
    }
}

#[derive(Debug)]
pub struct X11FullscreenBackend {
    #[cfg(unix)]
    conn: RustConnection,
    #[cfg(unix)]
    root: xproto::Window,
    #[cfg(unix)]
    image_byte_order: xproto::ImageOrder,
    #[cfg(unix)]
    pixmap_format: PixmapFormatInfo,
    #[cfg(unix)]
    shm: Option<ShmCapture>,
    width: u16,
    height: u16,
    visual_masks: Option<(u32, u32, u32)>,
    title: String,
}

impl X11FullscreenBackend {
    pub fn connect(title: impl Into<String>) -> Result<Self> {
        #[cfg(unix)]
        {
            let (conn, screen_num) = RustConnection::connect(None).location(loc!())?;
            let (width, height, visual_masks, root, image_byte_order, pixmap_format) = {
                let setup = conn.setup();
                let screen = &setup.roots[screen_num];
                let (red, green, blue) = find_visual_masks(setup, screen.root_visual);
                (
                    screen.width_in_pixels,
                    screen.height_in_pixels,
                    Some((red, green, blue)),
                    screen.root,
                    setup.image_byte_order,
                    PixmapFormatInfo::for_depth(setup, screen.root_depth, screen.width_in_pixels)
                        .location(loc!())?,
                )
            };

            // Allocate one extra scanline like xpra does, so consumers that operate in
            // scanline-sized chunks can safely read the last scanline without special
            // casing the bounds.
            let shm_segment_size = pixmap_format.bytes_per_line * (height as usize + 1);
            let shm = match ShmCapture::create(
                &conn,
                shm_segment_size,
                pixmap_format.bytes_per_line,
                pixmap_format.bytes_per_pixel,
            ) {
                Ok(shm) => shm,
                Err(err) => {
                    tracing::debug!(?err, "failed to initialize MIT-SHM; falling back to GetImage");
                    None
                }
            };
            Ok(Self {
                conn,
                root,
                image_byte_order,
                pixmap_format,
                shm,
                width,
                height,
                visual_masks,
                title: title.into(),
            })
        }

        #[cfg(not(unix))]
        {
            let _ = title;
            bail!("X11 backend is only supported on Unix")
        }
    }

    fn surface_state(&self) -> SurfaceState {
        SurfaceState {
            client: ClientId(1),
            id: WlSurfaceId(1),
            buffer: Some(BufferAssignment::New(Buffer {
                metadata: BufferMetadata {
                    width: self.width as i32,
                    height: self.height as i32,
                    stride: self.width as i32 * 4,
                    format: BufferFormat::Argb8888,
                },
                data: BufferData::External,
            })),
            role: Some(Role::XdgToplevel(XdgToplevelState {
                id: XdgToplevelId(1),
                parent: None,
                title: Some(self.title.clone()),
                app_id: Some("x11-fullscreen".to_string()),
                decoration_mode: None,
                maximized: None,
                fullscreen: None,
            })),
            buffer_scale: 1,
            buffer_transform: None,
            opaque_region: None,
            input_region: None,
            z_ordered_children: Vec::new(),
            damage: None,
            output_ids: Vec::new(),
            viewport_state: None,
            xdg_surface_state: None,
        }
    }

    #[cfg(unix)]
    fn capture_root_bgra(&mut self) -> Result<Vec<u8>> {
        let (red_mask, green_mask, blue_mask) = self
            .visual_masks
            .unwrap_or((0x00FF0000, 0x0000FF00, 0x000000FF));

        if let Some(shm) = &self.shm {
            let result = self
                .conn
                .shm_get_image(
                    self.root,
                    0,
                    0,
                    self.width,
                    self.height,
                    u32::MAX,
                    xproto::ImageFormat::Z_PIXMAP.into(),
                    shm.shmseg,
                    0,
                )
                .location(loc!())?
                .reply()
                .location(loc!());
            if let Err(err) = result {
                tracing::debug!(?err, "MIT-SHM capture failed; detaching and falling back");
                if let Some(shm) = self.shm.take() {
                    let _ = shm.detach(&self.conn);
                }
            }
        }

        if let Some(shm) = &self.shm {
            return shm_capture_to_bgra(
                shm,
                self.width,
                self.height,
                self.image_byte_order,
                red_mask,
                green_mask,
                blue_mask,
            )
            .location(loc!());
        }

        let reply = self
            .conn
            .get_image(
                xproto::ImageFormat::Z_PIXMAP,
                self.root,
                0,
                0,
                self.width,
                self.height,
                u32::MAX,
            )
            .location(loc!())?
            .reply()
            .location(loc!())?;

        get_image_reply_to_bgra(
            &reply.data,
            self.width,
            self.height,
            self.pixmap_format,
            self.image_byte_order,
            red_mask,
            green_mask,
            blue_mask,
        )
        .location(loc!())
    }
}

#[cfg(unix)]
impl Drop for X11FullscreenBackend {
    fn drop(&mut self) {
        if let Some(shm) = self.shm.take() {
            let _ = shm.detach(&self.conn);
        }
    }
}

impl PollingBackend for X11FullscreenBackend {
    fn capabilities(&self) -> Capabilities {
        Capabilities { xwayland: false }
    }

    fn initial_snapshot(&mut self) -> Result<Vec<SurfaceSnapshot>> {
        Ok(vec![SurfaceSnapshot {
            state: self.surface_state(),
        }])
    }

    fn poll(&mut self) -> Result<Vec<BackendObservation>> {
        #[cfg(unix)]
        {
            let bgra = self.capture_root_bgra().location(loc!())?;
            return Ok(vec![BackendObservation::SurfaceCommit {
                state: self.surface_state(),
                bgra: Some(Arc::from(bgra.into_boxed_slice())),
            }]);
        }

        #[cfg(not(unix))]
        {
            bail!("X11 backend is only supported on Unix")
        }
    }

    fn handle_client_event(&mut self, _event: Event) -> Result<()> {
        Ok(())
    }
}

#[cfg(unix)]
fn shm_capture_to_bgra(
    shm: &ShmCapture,
    width: u16,
    height: u16,
    image_byte_order: xproto::ImageOrder,
    red_mask: u32,
    green_mask: u32,
    blue_mask: u32,
) -> Result<Vec<u8>> {
    let pixel_count = width as usize * height as usize;
    let mut out = vec![0u8; pixel_count * 4];

    let frame_len = shm
        .bytes_per_line
        .checked_mul(height as usize)
        .ok_or_else(|| anyhow!("frame_len overflow"))?;
    ensure!(
        frame_len <= shm.map_len,
        "frame_len out of bounds: {frame_len} > {}",
        shm.map_len
    );

    let data = unsafe { std::slice::from_raw_parts(shm.map.as_ptr() as *const u8, frame_len) };
    blit_ximage_to_bgra(
        data,
        width,
        height,
        shm.bytes_per_line,
        shm.bytes_per_pixel,
        image_byte_order,
        red_mask,
        green_mask,
        blue_mask,
        &mut out,
    )?;

    Ok(out)
}

#[cfg(unix)]
fn get_image_reply_to_bgra(
    data: &[u8],
    width: u16,
    height: u16,
    pixmap_format: PixmapFormatInfo,
    image_byte_order: xproto::ImageOrder,
    red_mask: u32,
    green_mask: u32,
    blue_mask: u32,
) -> Result<Vec<u8>> {
    let pixel_count = width as usize * height as usize;
    let mut out = vec![0u8; pixel_count * 4];

    let height_usize = height as usize;
    ensure!(height_usize != 0, "height must be non-zero");
    ensure!(
        data.len() % height_usize == 0,
        "GetImage returned non-rectangular data: len={} height={height_usize}",
        data.len()
    );
    let bytes_per_line = data.len() / height_usize;
    ensure!(
        bytes_per_line >= width as usize * pixmap_format.bytes_per_pixel,
        "GetImage stride too small: stride={bytes_per_line} width={width} bpp={}",
        pixmap_format.bits_per_pixel
    );

    blit_ximage_to_bgra(
        data,
        width,
        height,
        bytes_per_line,
        pixmap_format.bytes_per_pixel,
        image_byte_order,
        red_mask,
        green_mask,
        blue_mask,
        &mut out,
    )?;

    Ok(out)
}

#[cfg(unix)]
fn blit_ximage_to_bgra(
    data: &[u8],
    width: u16,
    height: u16,
    bytes_per_line: usize,
    bytes_per_pixel: usize,
    image_byte_order: xproto::ImageOrder,
    red_mask: u32,
    green_mask: u32,
    blue_mask: u32,
    out_bgra: &mut [u8],
) -> Result<()> {
    ensure!(bytes_per_pixel >= 1 && bytes_per_pixel <= 4, "unsupported BPP: {bytes_per_pixel}");

    let width_usize = width as usize;
    let height_usize = height as usize;
    ensure!(out_bgra.len() == width_usize * height_usize * 4, "invalid output size");

    for y in 0..height_usize {
        let row = y
            .checked_mul(bytes_per_line)
            .ok_or_else(|| anyhow!("row offset overflow"))?;
        for x in 0..width_usize {
            let offset = row
                .checked_add(
                    x.checked_mul(bytes_per_pixel)
                        .ok_or_else(|| anyhow!("pixel offset overflow"))?,
                )
                .ok_or_else(|| anyhow!("pixel offset overflow"))?;
            ensure!(offset + bytes_per_pixel <= data.len(), "pixel read out of bounds");

            let pixel = read_pixel(data, offset, bytes_per_pixel, image_byte_order);

            let r = extract_channel(pixel, red_mask);
            let g = extract_channel(pixel, green_mask);
            let b = extract_channel(pixel, blue_mask);

            let i = (y * width_usize + x) * 4;
            out_bgra[i + 0] = b;
            out_bgra[i + 1] = g;
            out_bgra[i + 2] = r;
            out_bgra[i + 3] = 255;
        }
    }

    Ok(())
}

#[cfg(unix)]
fn read_pixel(
    data: &[u8],
    offset: usize,
    bytes_per_pixel: usize,
    image_byte_order: xproto::ImageOrder,
) -> u32 {
    match bytes_per_pixel {
        4 => {
            let bytes = [data[offset], data[offset + 1], data[offset + 2], data[offset + 3]];
            match image_byte_order {
                xproto::ImageOrder::LSB_FIRST => u32::from_le_bytes(bytes),
                xproto::ImageOrder::MSB_FIRST => u32::from_be_bytes(bytes),
                _ => u32::from_ne_bytes(bytes),
            }
        }
        3 => match image_byte_order {
            xproto::ImageOrder::LSB_FIRST => {
                u32::from(data[offset])
                    | (u32::from(data[offset + 1]) << 8)
                    | (u32::from(data[offset + 2]) << 16)
            }
            xproto::ImageOrder::MSB_FIRST => {
                (u32::from(data[offset]) << 16)
                    | (u32::from(data[offset + 1]) << 8)
                    | u32::from(data[offset + 2])
            }
            _ => {
                u32::from(data[offset])
                    | (u32::from(data[offset + 1]) << 8)
                    | (u32::from(data[offset + 2]) << 16)
            }
        },
        2 => {
            let bytes = [data[offset], data[offset + 1]];
            match image_byte_order {
                xproto::ImageOrder::LSB_FIRST => u32::from(u16::from_le_bytes(bytes)),
                xproto::ImageOrder::MSB_FIRST => u32::from(u16::from_be_bytes(bytes)),
                _ => u32::from(u16::from_ne_bytes(bytes)),
            }
        }
        1 => u32::from(data[offset]),
        _ => 0,
    }
}

fn compute_stride_bytes(width: u16, bits_per_pixel: u8, scanline_pad: u8) -> Option<usize> {
    let width_bits = (width as usize).checked_mul(bits_per_pixel as usize)?;
    let pad = scanline_pad as usize;
    if pad == 0 {
        return None;
    }
    let padded_bits = width_bits.checked_add(pad - 1)? / pad * pad;
    padded_bits.checked_div(8)
}

#[cfg(test)]
mod tests {
    use super::compute_stride_bytes;

    #[test]
    fn compute_stride_bytes_32bpp_32pad() {
        assert_eq!(compute_stride_bytes(1, 32, 32), Some(4));
        assert_eq!(compute_stride_bytes(2, 32, 32), Some(8));
        assert_eq!(compute_stride_bytes(1920, 32, 32), Some(7680));
    }

    #[test]
    fn compute_stride_bytes_24bpp_32pad_rounds_up() {
        // 24bpp uses 32-bit scanline padding on common servers.
        assert_eq!(compute_stride_bytes(1, 24, 32), Some(4));
        assert_eq!(compute_stride_bytes(2, 24, 32), Some(8));
        assert_eq!(compute_stride_bytes(3, 24, 32), Some(12));
    }
}

#[cfg(unix)]
fn find_visual_masks(
    setup: &x11rb::protocol::xproto::Setup,
    visual: xproto::Visualid,
) -> (u32, u32, u32) {
    for screen in &setup.roots {
        for depth in &screen.allowed_depths {
            for v in &depth.visuals {
                if v.visual_id == visual {
                    return (v.red_mask, v.green_mask, v.blue_mask);
                }
            }
        }
    }
    // Fallback to the common 0x00RRGGBB masks.
    (0x00FF0000, 0x0000FF00, 0x000000FF)
}

#[cfg(unix)]
fn extract_channel(pixel: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 0;
    }
    let shift = mask.trailing_zeros();
    let raw = (pixel & mask) >> shift;
    let bits = mask.count_ones();
    if bits >= 8 {
        (raw & 0xFF) as u8
    } else {
        // Scale to 8-bit.
        let max = (1u32 << bits) - 1;
        ((raw * 255 + max / 2) / max) as u8
    }
}
