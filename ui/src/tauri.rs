//! A small typed wrapper over Tauri's `invoke`, `listen` and the current
//! window's title bar controls (`window.__TAURI__`, `app.withGlobalTauri`).

use std::fmt;

use ipc::command::{Command, Log, LogArgs, LogLevel};
use ipc::{Event, IpcError};
use serde::Deserialize;
use wasm_bindgen::JsValue;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(catch, js_namespace = ["window", "__TAURI__", "core"], js_name = invoke)]
    fn tauri_invoke(cmd: &str, args: JsValue) -> Result<js_sys::Promise, JsValue>;

    #[wasm_bindgen(catch, js_namespace = ["window", "__TAURI__", "event"], js_name = listen)]
    fn tauri_listen(
        event: &str,
        handler: &Closure<dyn FnMut(JsValue)>,
    ) -> Result<js_sys::Promise, JsValue>;

    /// The webview's own window (`@tauri-apps/api/window`).
    type TauriWindow;

    #[wasm_bindgen(catch, js_namespace = ["window", "__TAURI__", "window"], js_name = getCurrentWindow)]
    fn current_window() -> Result<TauriWindow, JsValue>;

    #[wasm_bindgen(method, catch)]
    fn minimize(this: &TauriWindow) -> Result<js_sys::Promise, JsValue>;

    #[wasm_bindgen(method, catch, js_name = toggleMaximize)]
    fn toggle_maximize(this: &TauriWindow) -> Result<js_sys::Promise, JsValue>;

    #[wasm_bindgen(method, catch)]
    fn close(this: &TauriWindow) -> Result<js_sys::Promise, JsValue>;

    #[wasm_bindgen(method, catch, js_name = isMaximized)]
    fn is_maximized(this: &TauriWindow) -> Result<js_sys::Promise, JsValue>;

    #[wasm_bindgen(method, catch, js_name = onResized)]
    fn on_resized(
        this: &TauriWindow,
        handler: &Closure<dyn FnMut(JsValue)>,
    ) -> Result<js_sys::Promise, JsValue>;
}

/// Why an `invoke` failed.
#[derive(Debug)]
pub enum InvokeError {
    /// The command returned an error.
    Command(IpcError),
    /// IPC itself failed (unknown command, bad arguments, (de)serialization);
    /// a bug, for the log only.
    Ipc(String),
}

impl fmt::Display for InvokeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Command(e) => write!(f, "command failed: {e}"),
            Self::Ipc(e) => write!(f, "IPC failed: {e}"),
        }
    }
}

fn ipc_error(value: impl fmt::Debug) -> InvokeError {
    InvokeError::Ipc(format!("{value:?}"))
}

fn serialize<T: serde::Serialize>(value: &T) -> Result<JsValue, InvokeError> {
    value
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .map_err(ipc_error)
}

/// Starts command `C` and returns its pending promise.
fn start<C: Command>(args: &C::Args) -> Result<js_sys::Promise, InvokeError> {
    tauri_invoke(C::NAME, serialize(args)?).map_err(ipc_error)
}

/// Runs command `C` in the core.
pub async fn invoke<C: Command>(args: &C::Args) -> Result<C::Output, InvokeError> {
    match JsFuture::from(start::<C>(args)?).await {
        Ok(value) => serde_wasm_bindgen::from_value(value).map_err(ipc_error),
        Err(error) => Err(
            match serde_wasm_bindgen::from_value::<IpcError>(error.clone()) {
                Ok(error) => InvokeError::Command(error),
                Err(_) => ipc_error(error),
            },
        ),
    }
}

/// Sends `message` to the core's log without waiting; usable from the panic
/// hook, where no future may run.
pub fn log(level: LogLevel, message: String) {
    // Nothing is left to report a failure to.
    let _ = start::<Log>(&LogArgs { level, message });
}

/// What Tauri passes to an event handler.
#[derive(Deserialize)]
struct EventMessage {
    payload: Event,
}

/// Calls `handler` with every [`Event`] emitted under `name`, for the
/// lifetime of the window.
pub async fn listen(
    name: &str,
    mut handler: impl FnMut(Event) + 'static,
) -> Result<(), InvokeError> {
    let closure = Closure::<dyn FnMut(JsValue)>::new(move |message: JsValue| {
        match serde_wasm_bindgen::from_value::<EventMessage>(message) {
            Ok(message) => handler(message.payload),
            Err(error) => log(LogLevel::Error, format!("undecodable event: {error}")),
        }
    });
    let promise = tauri_listen(name, &closure).map_err(ipc_error)?;
    JsFuture::from(promise).await.map_err(ipc_error)?;
    // Subscribed until the window closes.
    closure.forget();
    Ok(())
}

/// Runs command `C` without waiting for it; a failure goes to the core's log.
pub fn send<C: Command>(args: C::Args)
where
    C::Args: 'static,
{
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(error) = invoke::<C>(&args).await {
            log(LogLevel::Error, format!("{}: {error}", C::NAME));
        }
    });
}

/// A title bar control of the current window (F15).
#[derive(Clone, Copy, Debug)]
pub enum WindowControl {
    Minimize,
    ToggleMaximize,
    /// Asks the core to close the window; it applies the exit action.
    Close,
}

/// Applies `control` to the current window; a failure goes to the core's log.
pub fn window_control(control: WindowControl) {
    let started = current_window().and_then(|window| match control {
        WindowControl::Minimize => window.minimize(),
        WindowControl::ToggleMaximize => window.toggle_maximize(),
        WindowControl::Close => window.close(),
    });
    wasm_bindgen_futures::spawn_local(async move {
        let done = match started {
            Ok(promise) => JsFuture::from(promise).await,
            Err(error) => Err(error),
        };
        if let Err(error) = done {
            log(LogLevel::Error, format!("{control:?}: {error:?}"));
        }
    });
}

/// Calls `handler` with whether the current window is maximized, now and
/// after every resize, for the lifetime of the window.
pub async fn watch_maximized(handler: impl Fn(bool) + 'static) -> Result<(), InvokeError> {
    async fn is_maximized() -> Result<bool, InvokeError> {
        let window = current_window().map_err(ipc_error)?;
        let value = JsFuture::from(window.is_maximized().map_err(ipc_error)?)
            .await
            .map_err(ipc_error)?;
        Ok(value.as_bool().unwrap_or_default())
    }
    let handler = std::rc::Rc::new(handler);
    handler(is_maximized().await?);
    let on_resize = std::rc::Rc::clone(&handler);
    let closure = Closure::<dyn FnMut(JsValue)>::new(move |_: JsValue| {
        let handler = std::rc::Rc::clone(&on_resize);
        wasm_bindgen_futures::spawn_local(async move {
            match is_maximized().await {
                Ok(maximized) => handler(maximized),
                Err(error) => log(LogLevel::Error, format!("isMaximized: {error}")),
            }
        });
    });
    let window = current_window().map_err(ipc_error)?;
    JsFuture::from(window.on_resized(&closure).map_err(ipc_error)?)
        .await
        .map_err(ipc_error)?;
    // Subscribed until the window closes.
    closure.forget();
    Ok(())
}
