//! Win32 composition order for the GPU video and transparent controls.
//! A transparent sibling still owns a rectangular HWND. Clipping the video's
//! visible region against that sibling prevents DWM from compositing its pixels.
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetParent, GetWindow, GetWindowLongPtrW, IsWindow, SetWindowLongPtrW, SetWindowPos, GWL_STYLE,
    GW_CHILD, GW_HWNDNEXT, HWND_TOP, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    WS_CLIPSIBLINGS,
};

pub(crate) fn restack(video: usize, overlays: &[usize]) -> Result<(), String> {
    let video = video as HWND;
    unsafe {
        if IsWindow(video) == 0 {
            return Err("视频窗口已关闭".into());
        }
        let parent = GetParent(video);
        if parent.is_null()
            || overlays.is_empty()
            || overlays
                .iter()
                .any(|&hwnd| IsWindow(hwnd as HWND) == 0 || GetParent(hwnd as HWND) != parent)
        {
            return Err("播放器图层不属于同一窗口".into());
        }
        let style = GetWindowLongPtrW(video, GWL_STYLE);
        let clipped = style & WS_CLIPSIBLINGS as isize != 0;
        if clipped {
            SetWindowLongPtrW(video, GWL_STYLE, style & !(WS_CLIPSIBLINGS as isize));
        }
        let flags = SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE;
        // Reassert the complete order: ordinary app WebView, then video, then
        // transparent controls. No global topmost windows or focus changes.
        if SetWindowPos(
            video,
            HWND_TOP,
            0,
            0,
            0,
            0,
            flags | if clipped { SWP_FRAMECHANGED } else { 0 },
        ) == 0
        {
            return Err("视频图层排序失败".into());
        }
        for &overlay in overlays {
            if SetWindowPos(overlay as HWND, HWND_TOP, 0, 0, 0, 0, flags) == 0 {
                return Err("弹幕图层排序失败".into());
            }
        }
    }
    if valid(video as usize, overlays) {
        Ok(())
    } else {
        Err("播放器图层顺序未生效".into())
    }
}

/// Structural evidence only; the release diagnostic separately checks pixels.
pub(crate) fn valid(video: usize, overlays: &[usize]) -> bool {
    unsafe {
        let parent = GetParent(video as HWND);
        if parent.is_null()
            || overlays.is_empty()
            || GetWindowLongPtrW(video as HWND, GWL_STYLE) & WS_CLIPSIBLINGS as isize != 0
        {
            return false;
        }
        let mut order = Vec::new();
        let mut child = GetWindow(parent, GW_CHILD);
        while !child.is_null() {
            order.push(child as usize);
            child = GetWindow(child, GW_HWNDNEXT);
        }
        let Some(video_index) = order.iter().position(|&hwnd| hwnd == video) else {
            return false;
        };
        overlays.iter().all(|overlay| {
            order
                .iter()
                .position(|hwnd| hwnd == overlay)
                .is_some_and(|index| index < video_index)
        }) && order
            .iter()
            .enumerate()
            .all(|(index, hwnd)| *hwnd == video || overlays.contains(hwnd) || index > video_index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, WS_CHILD, WS_CLIPCHILDREN,
    };
    struct Window(HWND);
    impl Drop for Window {
        fn drop(&mut self) {
            unsafe {
                DestroyWindow(self.0);
            }
        }
    }
    fn window(parent: HWND, style: u32) -> Window {
        let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                std::ptr::null(),
                style,
                0,
                0,
                640,
                360,
                parent,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        assert!(!hwnd.is_null());
        Window(hwnd)
    }
    #[test]
    fn restores_real_child_z_order_after_main_webview_raises_and_removes_video_clip() {
        let parent = window(std::ptr::null_mut(), 0);
        let main = window(parent.0, WS_CHILD);
        let video = window(parent.0, WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS);
        let overlay = window(parent.0, WS_CHILD | WS_CLIPCHILDREN);
        for raised in [main.0, video.0, overlay.0, main.0] {
            unsafe {
                SetWindowPos(
                    raised,
                    HWND_TOP,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
                );
            }
            restack(video.0 as usize, &[overlay.0 as usize]).unwrap();
            assert!(valid(video.0 as usize, &[overlay.0 as usize]));
            assert_eq!(unsafe { GetWindow(parent.0, GW_CHILD) }, overlay.0);
            assert_eq!(unsafe { GetWindow(overlay.0, GW_HWNDNEXT) }, video.0);
            assert_eq!(unsafe { GetWindow(video.0, GW_HWNDNEXT) }, main.0);
            assert_ne!(
                unsafe { GetWindowLongPtrW(video.0, GWL_STYLE) } & WS_CLIPCHILDREN as isize,
                0
            );
        }
    }
    #[test]
    fn rejects_overlay_from_another_parent_without_reparenting() {
        let first = window(std::ptr::null_mut(), 0);
        let second = window(std::ptr::null_mut(), 0);
        let video = window(first.0, WS_CHILD);
        let overlay = window(second.0, WS_CHILD);
        assert!(restack(video.0 as usize, &[overlay.0 as usize]).is_err());
        assert_eq!(unsafe { GetParent(overlay.0) }, second.0);
    }
}
