use tauri::{
    App, AppHandle, Manager,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

const OPEN: &str = "open-sanser";
const QUIT: &str = "quit-sanser";

pub fn configure_background_timers(config: &mut tauri::Config) {
    // Host heartbeat and incoming requests currently live in the webview.
    // WebView2 must keep their timers running when the main window is hidden.
    // This flag does not keep the computer awake or enable hosting by itself.
    if let Some(window) = config
        .app
        .windows
        .iter_mut()
        .find(|window| window.label == "main")
    {
        let args = window.additional_browser_args.get_or_insert_default();
        if !args
            .split_whitespace()
            .any(|arg| arg == "--disable-background-timer-throttling")
        {
            args.push_str(" --disable-background-timer-throttling");
        }
    }
}

fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let result = window
            .show()
            .and_then(|()| window.unminimize())
            .and_then(|()| window.set_focus());
        if let Err(error) = result {
            eprintln!("Unable to reopen Sanser from the system tray: {error}");
        }
    }
}

pub fn install(app: &App) -> Result<(), Box<dyn std::error::Error>> {
    let open = MenuItem::with_id(app, OPEN, "Open Sanser", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, QUIT, "Quit Sanser", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &separator, &quit])?;
    let icon = app
        .default_window_icon()
        .ok_or("Sanser tray icon is missing")?;
    TrayIconBuilder::with_id("sanser-tray")
        .icon(icon.clone())
        .tooltip("Sanser — running in the background")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            OPEN => show_main(app),
            QUIT => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                show_main(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}
