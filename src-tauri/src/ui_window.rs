use std::collections::HashMap;
use std::sync::Mutex;
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, PhysicalPosition, PhysicalSize,
    Rect, State, Webview, WebviewBuilder, WebviewUrl, Window, WindowEvent,
};

pub struct UiWebviewStore {
    restore_bounds: Mutex<HashMap<String, Rect>>,
}

impl UiWebviewStore {
    pub fn new() -> Self {
        Self {
            restore_bounds: Mutex::new(HashMap::new()),
        }
    }
}

fn validate_local_url(url: &str) -> Result<tauri::Url, String> {
    let parsed: tauri::Url = url.parse().map_err(|e| format!("Invalid UI URL: {e}"))?;
    let allowed_host = parsed
        .host_str()
        .map(|host| host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "::1")
        .unwrap_or(false);
    if !matches!(parsed.scheme(), "http" | "https") || !allowed_host {
        return Err("Only local HTTP UI URLs are allowed".to_string());
    }
    Ok(parsed)
}

fn webview_label(agent_id: &str) -> String {
    let safe_id: String = agent_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' {
                ch
            } else {
                '-'
            }
        })
        .collect();
    format!("agent-ui-{safe_id}")
}

fn fullscreen_window_label(agent_id: &str) -> String {
    format!("{}-fullscreen", webview_label(agent_id))
}

fn fill_window(webview: &Webview, window: &Window) -> Result<(), String> {
    let size = window.inner_size().map_err(|e| e.to_string())?;
    webview
        .set_bounds(Rect {
            position: PhysicalPosition::new(0, 0).into(),
            size: PhysicalSize::new(size.width, size.height).into(),
        })
        .map_err(|e| e.to_string())
}

fn restore_agent_ui_webview(app: &AppHandle, agent_id: &str) -> Result<(), String> {
    let Some(webview) = app.get_webview(&webview_label(agent_id)) else {
        return Ok(());
    };
    let main_window = app
        .get_window("main")
        .ok_or_else(|| "Main window not found".to_string())?;
    let bounds = app
        .state::<UiWebviewStore>()
        .restore_bounds
        .lock()
        .map_err(|e| e.to_string())?
        .remove(agent_id);

    webview.set_auto_resize(false).map_err(|e| e.to_string())?;
    webview.reparent(&main_window).map_err(|e| e.to_string())?;
    if let Some(bounds) = bounds {
        webview.set_bounds(bounds).map_err(|e| e.to_string())?;
    }
    webview.show().map_err(|e| e.to_string())?;
    let _ = app.emit("agent-ui-fullscreen-closed", agent_id.to_string());
    Ok(())
}

#[tauri::command]
pub async fn open_agent_ui_webview(
    agent_id: String,
    url: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    app: AppHandle,
) -> Result<(), String> {
    let parsed = validate_local_url(&url)?;
    let label = webview_label(&agent_id);
    let position = LogicalPosition::new(x.max(0.0), y.max(0.0));
    let size = LogicalSize::new(width.max(1.0), height.max(1.0));

    if let Some(webview) = app.get_webview(&label) {
        webview.navigate(parsed).map_err(|e| e.to_string())?;
        webview.set_position(position).map_err(|e| e.to_string())?;
        webview.set_size(size).map_err(|e| e.to_string())?;
        webview.show().map_err(|e| e.to_string())?;
        webview.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
    }

    let window = app
        .get_window("main")
        .ok_or_else(|| "Main window not found".to_string())?;
    window
        .add_child(
            WebviewBuilder::new(label, WebviewUrl::External(parsed)),
            position,
            size,
        )
        .map_err(|e| format!("Failed to embed UI: {e}"))?;
    Ok(())
}

#[tauri::command]
pub fn update_agent_ui_webview(
    agent_id: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    visible: bool,
    store: State<'_, UiWebviewStore>,
    app: AppHandle,
) -> Result<(), String> {
    if store
        .restore_bounds
        .lock()
        .map_err(|e| e.to_string())?
        .contains_key(&agent_id)
    {
        return Ok(());
    }
    let Some(webview) = app.get_webview(&webview_label(&agent_id)) else {
        return Ok(());
    };
    if visible {
        webview
            .set_position(LogicalPosition::new(x.max(0.0), y.max(0.0)))
            .map_err(|e| e.to_string())?;
        webview
            .set_size(LogicalSize::new(width.max(1.0), height.max(1.0)))
            .map_err(|e| e.to_string())?;
        webview.show().map_err(|e| e.to_string())?;
    } else {
        webview.hide().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn fullscreen_agent_ui_webview(
    agent_id: String,
    title: String,
    store: State<'_, UiWebviewStore>,
    app: AppHandle,
) -> Result<(), String> {
    let webview = app
        .get_webview(&webview_label(&agent_id))
        .ok_or_else(|| "Agent UI is not open".to_string())?;
    let bounds = webview.bounds().map_err(|e| e.to_string())?;
    store
        .restore_bounds
        .lock()
        .map_err(|e| e.to_string())?
        .insert(agent_id.clone(), bounds);

    let window_label = fullscreen_window_label(&agent_id);
    let fullscreen_window = if let Some(window) = app.get_window(&window_label) {
        window
    } else {
        let window = tauri::window::WindowBuilder::new(&app, window_label)
            .title(format!("{title} - UI"))
            .inner_size(1200.0, 800.0)
            .min_inner_size(720.0, 500.0)
            .visible(false)
            .build()
            .map_err(|e| format!("Failed to create fullscreen window: {e}"))?;

        let close_app = app.clone();
        let close_agent_id = agent_id.clone();
        let close_window = window.clone();
        window.on_window_event(move |event| match event {
            WindowEvent::Resized(size) => {
                if let Some(webview) = close_app.get_webview(&webview_label(&close_agent_id)) {
                    let _ = webview.set_bounds(Rect {
                        position: PhysicalPosition::new(0, 0).into(),
                        size: PhysicalSize::new(size.width, size.height).into(),
                    });
                }
            }
            WindowEvent::ScaleFactorChanged { new_inner_size, .. } => {
                if let Some(webview) = close_app.get_webview(&webview_label(&close_agent_id)) {
                    let _ = webview.set_bounds(Rect {
                        position: PhysicalPosition::new(0, 0).into(),
                        size: PhysicalSize::new(new_inner_size.width, new_inner_size.height).into(),
                    });
                }
            }
            WindowEvent::Focused(true) => {
                if let Some(webview) = close_app.get_webview(&webview_label(&close_agent_id)) {
                    let _ = fill_window(&webview, &close_window);
                }
            }
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = restore_agent_ui_webview(&close_app, &close_agent_id);
                let _ = close_window.hide();
            }
            _ => {}
        });
        window
    };

    webview.hide().map_err(|e| e.to_string())?;
    webview.set_auto_resize(false).map_err(|e| e.to_string())?;
    webview
        .reparent(&fullscreen_window)
        .map_err(|e| e.to_string())?;
    fill_window(&webview, &fullscreen_window)?;
    fullscreen_window.show().map_err(|e| e.to_string())?;
    fullscreen_window.set_focus().map_err(|e| e.to_string())?;
    fill_window(&webview, &fullscreen_window)?;
    webview.show().map_err(|e| e.to_string())?;
    webview.set_focus().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn close_agent_ui_webview(agent_id: String, app: AppHandle) -> Result<(), String> {
    app.state::<UiWebviewStore>()
        .restore_bounds
        .lock()
        .map_err(|e| e.to_string())?
        .remove(&agent_id);
    if let Some(window) = app.get_window(&fullscreen_window_label(&agent_id)) {
        let _ = window.hide();
    }
    if let Some(webview) = app.get_webview(&webview_label(&agent_id)) {
        webview.close().map_err(|e| e.to_string())?;
    }
    Ok(())
}
