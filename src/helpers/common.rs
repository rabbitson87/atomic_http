pub fn get_static_str(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}
