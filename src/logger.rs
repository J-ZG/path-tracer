/// Sends a [`web_sys::CustomEvent`] to the JavaScript window object.
///
/// `detail` is converted to a [`wasm_bindgen::JsValue`].
#[cfg(target_arch = "wasm32")]
pub(super) fn send_js_event(event_name: &str, detail: &str) {
    if let Some(window) = web_sys::window() {
        let init = web_sys::CustomEventInit::new();
        init.set_detail(&wasm_bindgen::JsValue::from_str(detail));
        if let Ok(event) = web_sys::CustomEvent::new_with_event_init_dict(event_name, &init) {
            let _ = window.dispatch_event(&event);
        }
    }
}

/// For implementing [`log::Log`] for the web.
#[cfg(target_arch = "wasm32")]
struct WebLogger {
    max_level: log::LevelFilter,
}

#[cfg(target_arch = "wasm32")]
impl log::Log for WebLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level().to_level_filter() <= self.max_level
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            let timestamp = js_sys::Date::new_0().to_iso_string();

            let msg = format!(
                "[{} {:<5} {}] {}",
                timestamp,
                record.level(),
                record.target(),
                record.args(),
            ); // matches env_logger formatting on native (except no colors)

            let js_msg = wasm_bindgen::JsValue::from_str(&msg);

            match record.level() {
                log::Level::Error => web_sys::console::error_1(&js_msg),
                log::Level::Warn => web_sys::console::warn_1(&js_msg),
                log::Level::Info => web_sys::console::info_1(&js_msg),
                _ => web_sys::console::log_1(&js_msg),
            }

            send_js_event("rust-log", &msg); // hence, JS knows about every log
        }
    }

    fn flush(&self) {}
}

/// Initialize logging.
///
/// On native, initializes `env_logger`. On the web, initializes custom logger `WebLogger`.
pub(super) fn init_environment(max_level: log::LevelFilter) {
    #[cfg(target_arch = "wasm32")]
    {
        std::panic::set_hook(Box::new(|panic_info| {
            let msg = panic_info.to_string();
            web_sys::console::error_1(&wasm_bindgen::JsValue::from_str(&msg));
            send_js_event("rust-crash", &msg); // hence on every panic, error is logged to console and JS knows about it
        }));

        let _ = log::set_boxed_logger(Box::new(WebLogger { max_level }));
        log::set_max_level(max_level);
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        env_logger::builder()
            .format_timestamp_millis()
            .filter_level(max_level)
            .init();
    }

    log::info!("Logging enabled");
}
