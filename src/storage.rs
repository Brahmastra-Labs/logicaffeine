use wasm_bindgen::prelude::*;

const PROGRESS_KEY: &str = "logos_user_progress";

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = localStorage, js_name = getItem)]
    fn local_storage_get(key: &str) -> Option<String>;

    #[wasm_bindgen(js_namespace = localStorage, js_name = setItem)]
    fn local_storage_set(key: &str, value: &str);

    #[wasm_bindgen(js_namespace = localStorage, js_name = removeItem)]
    fn local_storage_remove(key: &str);
}

pub fn load_raw() -> Option<String> {
    local_storage_get(PROGRESS_KEY)
}

pub fn save_raw(json: &str) {
    local_storage_set(PROGRESS_KEY, json);
}

pub fn clear() {
    local_storage_remove(PROGRESS_KEY);
}
