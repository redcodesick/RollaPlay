use image::{DynamicImage, RgbaImage, codecs::jpeg::JpegEncoder, imageops::FilterType};
use std::{
    collections::HashSet,
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, Copy)]
pub struct ThumbnailOptions {
    pub width: u32,
    pub height: u32,
    pub quality: u8,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct CacheCleanup {
    pub files_removed: usize,
    pub bytes_removed: u64,
}

pub fn generate_jpeg(path: &Path, options: ThumbnailOptions) -> Result<Vec<u8>, String> {
    if !(64..=1024).contains(&options.width) || !(64..=1024).contains(&options.height) {
        return Err("Thumbnail dimensions must be between 64 and 1024 pixels".to_string());
    }
    if !(40..=95).contains(&options.quality) {
        return Err("Thumbnail quality must be between 40 and 95".to_string());
    }
    let (rgba, source_width, source_height) = native_rgba(path, options.width, options.height)?;
    let source = RgbaImage::from_raw(source_width, source_height, rgba)
        .ok_or_else(|| "Windows returned an invalid thumbnail bitmap".to_string())?;
    let resized = DynamicImage::ImageRgba8(source).resize_to_fill(
        options.width,
        options.height,
        FilterType::Lanczos3,
    );
    let mut encoded = Cursor::new(Vec::new());
    JpegEncoder::new_with_quality(&mut encoded, options.quality)
        .encode_image(&resized)
        .map_err(|_| "Could not encode the thumbnail".to_string())?;
    Ok(encoded.into_inner())
}

#[cfg(not(windows))]
fn native_rgba(_path: &Path, _width: u32, _height: u32) -> Result<(Vec<u8>, u32, u32), String> {
    Err("Native Windows Shell thumbnails are only available in the Windows desktop app".to_string())
}

#[cfg(windows)]
fn native_rgba(path: &Path, width: u32, height: u32) -> Result<(Vec<u8>, u32, u32), String> {
    use std::{mem::size_of, os::windows::ffi::OsStrExt};
    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::{RPC_E_CHANGED_MODE, SIZE},
            Graphics::Gdi::{
                BI_RGB, BITMAP, BITMAPINFO, DIB_RGB_COLORS, DeleteObject, GetDC, GetDIBits,
                GetObjectW, HGDIOBJ, ReleaseDC,
            },
            System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize, IBindCtx},
            UI::Shell::{IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_THUMBNAILONLY},
        },
    };

    struct Apartment(bool);
    impl Drop for Apartment {
        fn drop(&mut self) {
            if self.0 {
                unsafe { CoUninitialize() };
            }
        }
    }

    let initialize_result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    if initialize_result.is_err() && initialize_result != RPC_E_CHANGED_MODE {
        return Err("Could not initialize the Windows Shell thumbnail service".to_string());
    }
    let _apartment = Apartment(initialize_result.is_ok());
    let wide_path = path.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<_>>();
    let bind_context: Option<&IBindCtx> = None;
    let shell_item: IShellItemImageFactory = unsafe {
        SHCreateItemFromParsingName::<_, _, IShellItemImageFactory>(
            PCWSTR(wide_path.as_ptr()),
            bind_context,
        )
    }
    .map_err(|_| "Windows could not open this video for thumbnail extraction".to_string())?;
    let bitmap = unsafe {
        shell_item.GetImage(
            SIZE {
                cx: width as i32,
                cy: height as i32,
            },
            SIIGBF_THUMBNAILONLY,
        )
    }
    .map_err(|_| "Windows has no video thumbnail provider for this file".to_string())?;

    struct BitmapGuard(windows::Win32::Graphics::Gdi::HBITMAP);
    impl Drop for BitmapGuard {
        fn drop(&mut self) {
            let _ = unsafe { DeleteObject(HGDIOBJ(self.0.0)) };
        }
    }
    let _bitmap_guard = BitmapGuard(bitmap);

    let mut bitmap_info = BITMAP::default();
    let read = unsafe {
        GetObjectW(
            _bitmap_guard.0.clone().into(),
            size_of::<BITMAP>() as i32,
            Some(&mut bitmap_info as *mut _ as *mut _),
        )
    };
    if read < size_of::<BITMAP>() as i32 || bitmap_info.bmWidth <= 0 || bitmap_info.bmHeight <= 0 {
        return Err("Windows returned an invalid thumbnail bitmap".to_string());
    }
    let source_width = bitmap_info.bmWidth as u32;
    let source_height = bitmap_info.bmHeight as u32;
    let pixel_count = (source_width as usize)
        .checked_mul(source_height as usize)
        .and_then(|count| count.checked_mul(4))
        .filter(|count| *count <= 16 * 1024 * 1024)
        .ok_or_else(|| "Windows returned an oversized thumbnail bitmap".to_string())?;
    let mut bitmap_info_header = BITMAPINFO::default();
    bitmap_info_header.bmiHeader.biSize = size_of::<windows::Win32::Graphics::Gdi::BITMAPINFOHEADER>() as u32;
    bitmap_info_header.bmiHeader.biWidth = bitmap_info.bmWidth;
    bitmap_info_header.bmiHeader.biHeight = -bitmap_info.bmHeight;
    bitmap_info_header.bmiHeader.biPlanes = 1;
    bitmap_info_header.bmiHeader.biBitCount = 32;
    bitmap_info_header.bmiHeader.biCompression = BI_RGB.0;
    bitmap_info_header.bmiHeader.biSizeImage = pixel_count as u32;
    let device_context = unsafe { GetDC(None) };
    if device_context.is_invalid() {
        return Err("Could not create a Windows graphics context".to_string());
    }
    struct DeviceContextGuard(windows::Win32::Graphics::Gdi::HDC);
    impl Drop for DeviceContextGuard {
        fn drop(&mut self) {
            unsafe { ReleaseDC(None, self.0) };
        }
    }
    let _device_context_guard = DeviceContextGuard(device_context);
    let mut bgra = vec![0_u8; pixel_count];
    let copied_lines = unsafe {
        GetDIBits(
            device_context,
            _bitmap_guard.0.clone(),
            0,
            source_height,
            Some(bgra.as_mut_ptr() as *mut _),
            &mut bitmap_info_header,
            DIB_RGB_COLORS,
        )
    };
    if copied_lines != source_height as i32 {
        return Err("Could not read pixels from the Windows thumbnail".to_string());
    }
    for pixel in bgra.chunks_exact_mut(4) {
        pixel.swap(0, 2);
        if pixel[3] == 0 {
            pixel[3] = 255;
        }
    }
    Ok((bgra, source_width, source_height))
}

pub fn write_cache_file(cache_dir: &Path, file_name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    if Path::new(file_name).file_name().and_then(|name| name.to_str()) != Some(file_name) {
        return Err("Invalid thumbnail cache filename".to_string());
    }
    fs::create_dir_all(cache_dir).map_err(|_| "Could not create the thumbnail cache".to_string())?;
    let output_path = cache_dir.join(file_name);
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary_path = cache_dir.join(format!("{file_name}.{nonce}.tmp"));
    fs::write(&temporary_path, bytes).map_err(|_| "Could not write the thumbnail cache".to_string())?;
    if let Err(error) = fs::rename(&temporary_path, &output_path) {
        let _ = fs::remove_file(&temporary_path);
        return Err(format!("Could not finalize the thumbnail cache: {error}"));
    }
    Ok(output_path)
}

pub fn cleanup_cache(cache_dir: &Path, referenced_paths: &HashSet<PathBuf>) -> Result<CacheCleanup, String> {
    let entries = match fs::read_dir(cache_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(CacheCleanup::default()),
        Err(_) => return Err("Could not read the thumbnail cache".to_string()),
    };
    let mut result = CacheCleanup::default();
    for entry in entries {
        let entry = entry.map_err(|_| "Could not read the thumbnail cache".to_string())?;
        let path = entry.path();
        let metadata = match entry.metadata() {
            Ok(metadata) if metadata.is_file() => metadata,
            _ => continue,
        };
        if path.extension().and_then(|extension| extension.to_str()) == Some("tmp")
            || !referenced_paths.contains(&path)
        {
            if fs::remove_file(&path).is_ok() {
                result.files_removed += 1;
                result.bytes_removed += metadata.len();
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::{cleanup_cache, write_cache_file};
    use std::{collections::HashSet, fs, path::Path};

    #[test]
    fn cache_writer_rejects_path_traversal() {
        let directory = tempfile::tempdir().unwrap();
        assert!(write_cache_file(directory.path(), "../outside.jpg", b"not an image").is_err());
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "Set WMS_TEST_VIDEO to a video with a registered Windows Shell thumbnail provider"]
    fn windows_shell_extracts_and_encodes_a_video_thumbnail() {
        let path = std::env::var_os("WMS_TEST_VIDEO").expect("WMS_TEST_VIDEO must point to a test video");
        let bytes = super::generate_jpeg(
            Path::new(&path),
            super::ThumbnailOptions { width: 320, height: 180, quality: 82 },
        ).unwrap();
        assert!(bytes.starts_with(&[0xff, 0xd8]));
    }

    #[test]
    fn cleanup_keeps_referenced_cache_and_removes_orphans() {
        let directory = tempfile::tempdir().unwrap();
        let keep = write_cache_file(directory.path(), "keep.jpg", b"keep").unwrap();
        write_cache_file(directory.path(), "orphan.jpg", b"orphan").unwrap();
        fs::write(directory.path().join("partial.tmp"), b"partial").unwrap();
        let referenced = HashSet::from([keep.clone()]);
        let result = cleanup_cache(directory.path(), &referenced).unwrap();
        assert_eq!(result.files_removed, 2);
        assert_eq!(fs::read(keep).unwrap(), b"keep");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
