include!(concat!(env!("OUT_DIR"), "/generated_search_ui.rs"));

pub const fn html() -> &'static str {
    SEARCH_UI_HTML
}
