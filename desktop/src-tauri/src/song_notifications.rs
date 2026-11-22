//! Native song-completion notifications for the Windows *portable* studio.
//! The Tauri notification plugin requires an installed Windows application.
//! A Shell notification-area balloon is supported without a Start Menu shortcut.
//! The extra notification icon is temporary; the separate "Close to tray"
//! icon is managed by Tauri and is never removed here.

#[cfg(windows)]
const NOTIFICATION_ICON_ID: u32 = 0x59554532; // "YUE2", distinct from Tauri's normal tray icon
// Give Windows time to deliver/display the balloon before removing its icon.
// The notification itself is handed off to Windows; do not leave an unused icon
// parked in the tray for the lifetime of the portable application.
#[cfg(windows)]
const NOTIFICATION_ICON_LIFETIME: std::time::Duration = std::time::Duration::from_secs(20);
#[cfg(windows)]
static NOTIFICATION_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A newer completion extends the lifetime; an older cleanup worker must not
/// remove the icon while a newer song notification is being displayed.
#[cfg(windows)]
fn schedule_icon_cleanup(hwnd: isize) {
    use std::sync::atomic::Ordering;

    let generation = NOTIFICATION_GENERATION.fetch_add(1, Ordering::SeqCst).wrapping_add(1);
    std::thread::spawn(move || {
        std::thread::sleep(NOTIFICATION_ICON_LIFETIME);
        if NOTIFICATION_GENERATION.load(Ordering::SeqCst) != generation {
            return;
        }
        use windows_sys::Win32::UI::Shell::{Shell_NotifyIconW, NOTIFYICONDATAW, NIM_DELETE};
        let mut icon: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        icon.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        icon.hWnd = hwnd as _;
        icon.uID = NOTIFICATION_ICON_ID;
        // NIM_DELETE only targets the temporary icon ID, never the user's
        // optional "keep running in tray" icon.
        unsafe { Shell_NotifyIconW(NIM_DELETE, &icon); }
    });
}
#[tauri::command]
pub fn notify_song_ready(app: tauri::AppHandle, message: String) -> Result<(), String> {
    if message.trim().is_empty() {
        return Err("the notification message is empty".into());
    }
    #[cfg(windows)]
    {
        show_windows_balloon(&app, &message)
    }
    #[cfg(not(windows))]
    {
        let _ = (app, message);
        Err("Windows desktop notifications are unavailable on this platform".into())
    }
}

/// A terminating NUL and a length limit are required by NOTIFYICONDATAW.
/// Truncate at Unicode scalar boundaries, never in the middle of a surrogate pair.
#[cfg(windows)]
fn wide_field<const N: usize>(value: &str) -> [u16; N] {
    let mut output = [0u16; N];
    let mut cursor = 0;
    for ch in value.chars() {
        let mut pair = [0u16; 2];
        let encoded = ch.encode_utf16(&mut pair);
        if cursor + encoded.len() >= N {
            break;
        }
        output[cursor..cursor + encoded.len()].copy_from_slice(encoded);
        cursor += encoded.len();
    }
    output
}

#[cfg(windows)]
fn show_windows_balloon(app: &tauri::AppHandle, message: &str) -> Result<(), String> {
    use tauri::Manager;
    use windows_sys::Win32::UI::Shell::{
        Shell_NotifyIconW, NOTIFYICONDATAW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP,
        NIM_ADD, NIM_MODIFY, NIM_DELETE, NIIF_INFO,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{LoadIconW, IDI_APPLICATION, WM_APP};

    let window = app.get_webview_window("main").ok_or("the main studio window is not available")?;
    let hwnd = window.hwnd().map_err(|error| error.to_string())?.0;

    // The icon is used only to deliver the balloon. We remove it after a short
    // grace period, instead of leaving a useless icon in the notification area.
    let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = hwnd as _;
    data.uID = NOTIFICATION_ICON_ID;
    data.uFlags = NIF_ICON | NIF_TIP | NIF_MESSAGE;
    data.uCallbackMessage = WM_APP + 0x4A2;
    data.hIcon = unsafe { LoadIconW(std::ptr::null_mut(), IDI_APPLICATION) };
    data.szTip = wide_field("YuE2 Studio song notifications");
    if data.hIcon.is_null() {
        return Err("could not load the system notification icon".into());
    }

    // Adding an already-present icon can fail harmlessly; NIM_MODIFY still
    // reaches it. Trying NIM_ADD each time also heals an Explorer restart.
    unsafe { Shell_NotifyIconW(NIM_ADD, &data) };
    data.uFlags = NIF_INFO;
    data.szInfoTitle = wide_field("YuE2 Studio");
    data.szInfo = wide_field(message);
    data.dwInfoFlags = NIIF_INFO;
    if unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) } == 0 {
        let error = std::io::Error::last_os_error();
        // If NIM_ADD created an icon but NIM_MODIFY failed, don't strand it.
        unsafe { Shell_NotifyIconW(NIM_DELETE, &data); }
        return Err(format!("Windows declined the notification: {error}"));
    }
    schedule_icon_cleanup(hwnd as isize);
    Ok(())
}

#[cfg(all(test, windows))]
mod tests {
    use super::{wide_field, NOTIFICATION_ICON_LIFETIME, NOTIFICATION_ICON_ID};
    #[test]
    fn temporary_icon_has_a_bounded_lifetime_and_its_own_identifier() {
        assert!(NOTIFICATION_ICON_LIFETIME.as_secs() > 0);
        assert!(NOTIFICATION_ICON_LIFETIME.as_secs() <= 30);
        assert_eq!(NOTIFICATION_ICON_ID, 0x59554532);
    }
    #[test]
    fn notification_fields_are_terminated_and_preserve_unicode_characters() {
        let field = wide_field::<8>("ab🎵cd");
        assert_eq!(String::from_utf16_lossy(&field[..6]), "ab🎵cd");
        assert_eq!(field[6], 0);
        let truncated = wide_field::<5>("abcd🎵");
        assert_eq!(String::from_utf16_lossy(&truncated[..4]), "abcd");
    }
}
