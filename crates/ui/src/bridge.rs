//! JavaScript transport and DOM boundary.
use wasm_bindgen::prelude::wasm_bindgen;

#[wasm_bindgen]
extern "C" {
    #[cfg(target_arch = "wasm32")]
    #[wasm_bindgen(js_namespace = GalGearBridge)]
    pub(crate) fn local_date(seconds: f64) -> String;
    #[cfg(target_arch = "wasm32")]
    #[wasm_bindgen(js_namespace = GalGearBridge)]
    pub(crate) fn local_date_time(seconds: f64) -> String;
    #[cfg(not(test))]
    #[wasm_bindgen(js_namespace = GalGearBridge)]
    pub(crate) fn render(html: &str);
    #[cfg(not(test))]
    #[wasm_bindgen(js_namespace = GalGearBridge)]
    pub(crate) fn reset_scroll();
    #[cfg(not(test))]
    #[wasm_bindgen(js_namespace = GalGearBridge)]
    pub(crate) fn request(id: u32, command: &str, payload: &str);
    #[cfg(not(test))]
    #[wasm_bindgen(js_namespace = GalGearBridge)]
    pub(crate) fn schedule_toast_expiry(token: u32, milliseconds: u32);
    #[wasm_bindgen(js_namespace = GalGearBridge)]
    pub(crate) fn schedule_download_notice_expiry(token: u32, milliseconds: u32);
}

#[cfg(test)]
thread_local! {
    static REQUESTS: std::cell::RefCell<Vec<(u32, String, String)>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
pub(crate) fn request(id: u32, command: &str, payload: &str) {
    REQUESTS.with(|requests| {
        requests
            .borrow_mut()
            .push((id, command.into(), payload.into()))
    });
}

#[cfg(test)]
pub(crate) fn take_requests() -> Vec<(u32, String, String)> {
    REQUESTS.with(|requests| std::mem::take(&mut *requests.borrow_mut()))
}

#[cfg(test)]
pub(crate) fn schedule_toast_expiry(_token: u32, _milliseconds: u32) {}

#[cfg(test)]
pub(crate) fn reset_scroll() {}

// Native event-routing tests exercise state changes without a browser DOM.
#[cfg(test)]
pub(crate) fn render(_html: &str) {}
