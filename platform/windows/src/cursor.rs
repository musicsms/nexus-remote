use crate::{BackendErrorKind, BackendResult};

/// A bounded RGBA cursor image and its position in desktop coordinates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorSnapshot {
    pub visible: bool,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub hotspot_x: u32,
    pub hotspot_y: u32,
    pub rgba: Vec<u8>,
}

impl CursorSnapshot {
    pub const MAX_DIMENSION: u32 = 256;
    pub const MAX_RGBA_BYTES: usize = 256 * 256 * 4;

    /// Validates the bounded cursor shape before it reaches a native API.
    pub fn validate(&self) -> BackendResult<()> {
        if !self.visible && self.width == 0 && self.height == 0 && self.rgba.is_empty() {
            return Ok(());
        }

        if self.width == 0
            || self.height == 0
            || self.width > Self::MAX_DIMENSION
            || self.height > Self::MAX_DIMENSION
        {
            return Err(BackendErrorKind::InvalidCursorDimensions.into());
        }

        let expected_len = (self.width as usize)
            .checked_mul(self.height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(BackendErrorKind::CursorPayloadLength)?;

        if expected_len > Self::MAX_RGBA_BYTES || self.rgba.len() != expected_len {
            return Err(BackendErrorKind::CursorPayloadLength.into());
        }

        if self.visible && (self.hotspot_x >= self.width || self.hotspot_y >= self.height) {
            return Err(BackendErrorKind::HotspotOutOfBounds.into());
        }

        Ok(())
    }
}

/// Native cursor calls are deliberately expressed without Windows types.
trait NativeCursorApi {
    fn snapshot(&mut self) -> BackendResult<CursorSnapshot>;
}

/// Captures the current Windows cursor through a validated, private native adapter.
pub struct WindowsCursorSource {
    native: SystemCursorApi,
}

impl WindowsCursorSource {
    /// Creates a cursor source for the current interactive desktop.
    pub fn system() -> Self {
        Self {
            native: SystemCursorApi::new(),
        }
    }

    /// Captures position, visibility, hotspot, and a bounded RGBA cursor shape.
    pub fn snapshot(&mut self) -> BackendResult<CursorSnapshot> {
        let snapshot = self.native.snapshot()?;
        snapshot.validate()?;
        Ok(snapshot)
    }
}

/// The platform adapter remains private so Windows handle types never cross
/// the platform crate boundary.
#[cfg(windows)]
struct SystemCursorApi(native::WindowsCursorApi);

#[cfg(not(windows))]
struct SystemCursorApi;

impl SystemCursorApi {
    fn new() -> Self {
        #[cfg(windows)]
        {
            Self(native::WindowsCursorApi)
        }

        #[cfg(not(windows))]
        {
            Self
        }
    }
}

#[cfg(windows)]
impl NativeCursorApi for SystemCursorApi {
    fn snapshot(&mut self) -> BackendResult<CursorSnapshot> {
        self.0.snapshot()
    }
}

#[cfg(not(windows))]
impl NativeCursorApi for SystemCursorApi {
    fn snapshot(&mut self) -> BackendResult<CursorSnapshot> {
        Err(BackendErrorKind::UnsupportedPlatform.into())
    }
}

#[cfg(windows)]
mod native {
    use std::mem::size_of;

    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Gdi::{
        GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
        DIB_RGB_COLORS, HBITMAP, HDC,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetCursorInfo, GetIconInfo, CURSORINFO, CURSOR_SHOWING, ICONINFO,
    };

    use super::{BackendErrorKind, BackendResult, CursorSnapshot, NativeCursorApi};

    pub(super) struct WindowsCursorApi;

    impl NativeCursorApi for WindowsCursorApi {
        fn snapshot(&mut self) -> BackendResult<CursorSnapshot> {
            let mut cursor_info = CURSORINFO {
                cbSize: size_of::<CURSORINFO>() as u32,
                ..Default::default()
            };
            // SAFETY: `cursor_info` points to initialized storage with the
            // documented `cbSize` required by GetCursorInfo.
            unsafe { GetCursorInfo(&mut cursor_info) }
                .map_err(|_| BackendErrorKind::NativeFailure)?;

            if cursor_info.flags.0 & CURSOR_SHOWING.0 == 0 {
                return Ok(CursorSnapshot {
                    visible: false,
                    x: cursor_info.ptScreenPos.x,
                    y: cursor_info.ptScreenPos.y,
                    width: 0,
                    height: 0,
                    hotspot_x: 0,
                    hotspot_y: 0,
                    rgba: Vec::new(),
                });
            }

            let mut icon_info = ICONINFO::default();
            // SAFETY: GetCursorInfo supplied this borrowed cursor handle and
            // `icon_info` is valid output storage. GetIconInfo allocates the
            // returned bitmap handles, which `IconBitmaps` releases below.
            unsafe { GetIconInfo(cursor_info.hCursor, &mut icon_info) }
                .map_err(|_| BackendErrorKind::NativeFailure)?;
            let bitmaps = IconBitmaps::new(icon_info.hbmColor, icon_info.hbmMask);
            let (width, height, rgba) = rgba_from_color_bitmap(bitmaps.color)?;

            Ok(CursorSnapshot {
                visible: true,
                x: cursor_info.ptScreenPos.x,
                y: cursor_info.ptScreenPos.y,
                width,
                height,
                hotspot_x: icon_info.xHotspot,
                hotspot_y: icon_info.yHotspot,
                rgba,
            })
        }
    }

    struct IconBitmaps {
        color: HBITMAP,
        mask: HBITMAP,
    }

    impl IconBitmaps {
        const fn new(color: HBITMAP, mask: HBITMAP) -> Self {
            Self { color, mask }
        }
    }

    impl Drop for IconBitmaps {
        fn drop(&mut self) {
            // SAFETY: GetIconInfo documents ownership transfer of both bitmap
            // handles to its caller. They are each deleted exactly once here.
            unsafe {
                if !self.color.is_invalid() {
                    let _ = windows::Win32::Graphics::Gdi::DeleteObject(self.color);
                }
                if !self.mask.is_invalid() {
                    let _ = windows::Win32::Graphics::Gdi::DeleteObject(self.mask);
                }
            }
        }
    }

    struct ScreenDc(HDC);

    impl ScreenDc {
        fn acquire() -> BackendResult<Self> {
            // SAFETY: a null HWND requests the screen DC, which is released by
            // this type before returning to the caller.
            let dc = unsafe { GetDC(HWND::default()) };
            if dc.is_invalid() {
                return Err(BackendErrorKind::NativeFailure.into());
            }
            Ok(Self(dc))
        }
    }

    impl Drop for ScreenDc {
        fn drop(&mut self) {
            // SAFETY: this value was produced by GetDC with the same null HWND.
            unsafe {
                let _ = ReleaseDC(HWND::default(), self.0);
            }
        }
    }

    fn rgba_from_color_bitmap(bitmap: HBITMAP) -> BackendResult<(u32, u32, Vec<u8>)> {
        if bitmap.is_invalid() {
            // Monochrome cursors lack a color bitmap. Failing closed prevents
            // inventing a cursor shape with incorrect transparency semantics.
            return Err(BackendErrorKind::NativeFailure.into());
        }

        let mut native_bitmap = BITMAP::default();
        // SAFETY: `native_bitmap` is writable for exactly its declared size;
        // the bitmap handle was allocated by GetIconInfo and is still owned.
        let copied = unsafe {
            GetObjectW(
                bitmap,
                size_of::<BITMAP>() as i32,
                Some((&mut native_bitmap as *mut BITMAP).cast()),
            )
        };
        if copied != size_of::<BITMAP>() as i32 {
            return Err(BackendErrorKind::NativeFailure.into());
        }

        let width = u32::try_from(native_bitmap.bmWidth)
            .map_err(|_| BackendErrorKind::InvalidCursorDimensions)?;
        let height = u32::try_from(native_bitmap.bmHeight)
            .map_err(|_| BackendErrorKind::InvalidCursorDimensions)?;
        if width == 0
            || height == 0
            || width > CursorSnapshot::MAX_DIMENSION
            || height > CursorSnapshot::MAX_DIMENSION
        {
            return Err(BackendErrorKind::InvalidCursorDimensions.into());
        }
        let bytes = usize::try_from(width)
            .ok()
            .and_then(|width| width.checked_mul(height as usize))
            .and_then(|pixels| pixels.checked_mul(4))
            .filter(|bytes| *bytes <= CursorSnapshot::MAX_RGBA_BYTES)
            .ok_or(BackendErrorKind::CursorPayloadLength)?;

        let mut bitmap_info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                // Request a top-down DIB so the native buffer shares the
                // cursor coordinate origin used by CursorSnapshot.
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                biSizeImage: bytes as u32,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bgra = vec![0_u8; bytes];
        let dc = ScreenDc::acquire()?;
        // SAFETY: `bgra` owns `bytes` writable bytes, the header requests the
        // matching top-down 32-bit DIB, and the screen DC remains live here.
        let copied_rows = unsafe {
            GetDIBits(
                dc.0,
                bitmap,
                0,
                height,
                Some(bgra.as_mut_ptr().cast()),
                &mut bitmap_info,
                DIB_RGB_COLORS,
            )
        };
        if copied_rows != height as i32 {
            return Err(BackendErrorKind::NativeFailure.into());
        }

        for pixel in bgra.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
        Ok((width, height, bgra))
    }
}
