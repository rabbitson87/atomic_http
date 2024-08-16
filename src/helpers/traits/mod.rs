pub mod bytes;
pub mod http_request;
pub mod http_response;
pub mod http_stream;

use std::collections::HashMap;

pub trait GetHeaderChild {
    fn get_header_child(&self) -> HashMap<String, String>;
}

impl GetHeaderChild for &str {
    fn get_header_child(&self) -> HashMap<String, String> {
        let mut result: HashMap<String, String> = HashMap::new();
        self.split("; ").for_each(|item| {
            if item != "form-data" {
                let mut items = item.split("=\"");
                let key = items.next().unwrap();
                let value = items.next().unwrap().replace("\"", "");
                result.insert(key.into(), value);
            }
        });
        result
    }
}

pub trait StringUtil {
    fn copy_string(&self) -> String;
}

impl StringUtil for String {
    fn copy_string(&self) -> String {
        self.as_str().into()
    }
}
