use gjson::get;
use sha256::digest;
use once_cell::sync::OnceCell;
use std::collections::HashMap;
use serde::{Serialize, Deserialize};
use serde_json::Value;
use eyre::ContextCompat;

use fluvio_smartmodule::{
    smartmodule, Result, Record, RecordData,
    dataplane::smartmodule::{
        SmartModuleExtraParams, SmartModuleInitError
    },
    eyre
};

static SPEC: OnceCell<KeygenParams> = OnceCell::new();
const PARAM_NAME: &str = "spec";

#[derive(Debug, Serialize, Deserialize)]
pub struct KeygenParams {
    lookup: Vec<String>,
    key_name: String
}

/// Extract json values based on an array of dot notations:
///     [ "top.one", "top.two"]
pub fn extract_json_values(json: &str, lookup: Vec<String>) -> String {
    lookup
        .iter()
        .map(|item| get(json, item.as_str()))
        .map(|value| value.to_string())
        .collect::<Vec<String>>()
        .join("")
}

/// Ecapsulate sha256::digest in an API.
pub fn generate_key(input: String) -> String {
    digest(input)
}

/// Add keys to a json Value.
pub fn add_keys(v: &Value, fields: &HashMap<String, String>) -> Value {
    match v {
        Value::Object(m) => {
            let mut m = m.clone();
            for (k, v) in fields {
                m.insert(k.clone(), Value::String(v.clone()));
            }
            Value::Object(m)
        }
        v => v.clone(),
    }
}


/// Generate a new Key field for a JSON record
pub fn add_key_to_json_record(record: &Record, spec: &KeygenParams) -> Result<Value> {
    let record: &str = std::str::from_utf8(record.value.as_ref())?;
    let key_val = extract_json_values(record, spec.lookup.clone());

    let record_value: Value = serde_json::from_str(record)?;
    let result = add_keys(&record_value, &HashMap::from([
        (spec.key_name.clone(), generate_key(key_val))
    ]));

    Ok(result)
}

#[smartmodule(map)]
pub fn map(record: &Record) -> Result<(Option<RecordData>, RecordData)> {
    let key = record.key.clone();
    let spec = SPEC.get().wrap_err("spec is not initialized")?;

    let result = add_key_to_json_record(&record, &spec)?;

    Ok((key, serde_json::to_string(&result)?.into()))
}

#[smartmodule(init)]
fn init(params: SmartModuleExtraParams) -> Result<()> {
    if let Some(raw_spec) = params.get(PARAM_NAME) {
        match serde_json::from_str(raw_spec) {
            Ok(spec) => {
                SPEC.set(spec).expect("spec is already initialized");
                Ok(())
            }
            Err(err) => {
                eprintln!("unable to parse spec from params: {err:?}");
                Err(eyre!("cannot parse `spec` param: {:#?}", err))
            }
        }
    } else {
        Err(SmartModuleInitError::MissingParam(PARAM_NAME.to_string()).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static INPUT: &str = r#"{
        "name": {"first": "Tom", "last": "Anderson"},
        "id": 373443,
        "items": [
            {
                "pub_date": "Tue, 17 Apr 2023 14:59:04 GMT",
                "last_build_date": "Tue, 18 Apr 2023 15:00:01 GMT",
                "link": "https://example.com/456970",      
            },
            {
                "pub_date": "Tue, 17 Apr 2023 14:59:44 GMT",
                "last_build_date": "Tue, 18 Apr 2023 15:00:01 GMT",        
                "link": "https://example.com/3343",      
            },
        ],
        "pub_date": "Tue, 18 Apr 2023 18:59:04 GMT",
        "last_build_date": "Tue, 20 Apr 2023 15:00:01 GMT",
        "link": "https://example.com/3343",      
    }"#;

    #[test]
    fn extract_json_values_tests() {

        // digit
        let lookup = vec![
            "id".to_owned()
        ];
        let result = "373443";
        assert_eq!(result.to_owned(), extract_json_values(INPUT, lookup));

        // string
        let lookup = vec![
            "link".to_owned(),
        ];
        let result = r#"https://example.com/3343"#;
        assert_eq!(result.to_owned(), extract_json_values(INPUT, lookup));

        // nested string
        let lookup = vec![
            "name.last".to_owned(),
        ];
        let result = r#"Anderson"#;
        assert_eq!(result.to_owned(), extract_json_values(INPUT, lookup));

        // multiple strings
        let lookup = vec![
            "pub_date".to_owned(),
            "last_build_date".to_owned(),
        ];
        let result = r#"Tue, 18 Apr 2023 18:59:04 GMTTue, 20 Apr 2023 15:00:01 GMT"#;
        assert_eq!(result.to_owned(), extract_json_values(INPUT, lookup));

        // full key-value tree
        let lookup = vec![
            "name".to_owned(),
        ];
        let result = r#"{"first": "Tom", "last": "Anderson"}"#;
        assert_eq!(result.to_owned(), extract_json_values(INPUT, lookup));

        // full array tree
        let lookup = vec![
            "items".to_owned()
        ];
        let result = r#"[
            {
                "pub_date": "Tue, 17 Apr 2023 14:59:04 GMT",
                "last_build_date": "Tue, 18 Apr 2023 15:00:01 GMT",
                "link": "https://example.com/456970",      
            },
            {
                "pub_date": "Tue, 17 Apr 2023 14:59:44 GMT",
                "last_build_date": "Tue, 18 Apr 2023 15:00:01 GMT",        
                "link": "https://example.com/3343",      
            },
        ]"#;
        assert_eq!(result.to_owned(), extract_json_values(INPUT, lookup));

        // mixed
        let lookup = vec![
            "items.0.pub_date".to_owned(),
            "items.0.last_build_date".to_owned(),
            "link".to_owned()
        ];
        let result = r#"Tue, 17 Apr 2023 14:59:04 GMTTue, 18 Apr 2023 15:00:01 GMThttps://example.com/3343"#;
        assert_eq!(result.to_owned(), extract_json_values(INPUT, lookup));

        // invalid 
        let lookup = vec![
            "invalid".to_owned()
        ];
        let result = "";
        assert_eq!(result.to_owned(), extract_json_values(INPUT, lookup));
        
    }

    #[test]
    fn generate_key_tests() {
        // simple
        let input = "Tue, 17 Apr 2023 14:59:04 GMT";
        assert_eq!(
            generate_key(input.to_owned()),
            "ba021aa33e0ba9557bae32efc690cc1c162aa6c2a0c62cb8527dc8fe7d5ca8d7");

        let input = r#"["Sara","Alex","Jack"]"#;
        assert_eq!(
            generate_key(input.to_owned()),
            "0c5507584b9b6c163335cd626fca364a3a34835a71383111b492a2249a64535f");
    }

    #[test]

    fn add_keys_tests() {
        let input = r#"{
            "aaaa": "value1", 
            "bbbb": "value2"
        }"#;
        let keys =  HashMap::from([
            ("key".to_owned(), "0c5507584b9b6c163335cd626fca364a3a34835a71383111b492a2249a64535f".to_owned())
        ]);
        let expected = r#"{
            "aaaa": "value1", 
            "bbbb": "value2",
            "key": "0c5507584b9b6c163335cd626fca364a3a34835a71383111b492a2249a64535f"
        }"#;
        let json_input:Value = serde_json::from_str(input).unwrap();
        let json_expected:Value = serde_json::from_str(expected).unwrap();

        let result = add_keys(&json_input, &keys);
        assert_eq!(result, json_expected);
    }

    #[test]
    fn add_key_to_json_record_tests() {
        let input = r#"{
            "last_build_date": "Tue, 18 Apr 2023 15:00:01 GMT",
            "description": "This is the description of my JSON object",
            "link": "http://www.example.com",
            "pub_date": "Mon, 17 Apr 2023 16:08:23 GMT",
            "title": "My Json Object Title"
        }"#;
        let expected = r#"{
            "dedup_key": "3193200642d322d171dd4c05875741ff7a4fc0f7a467b52d514d5ce273d4f762",
            "last_build_date": "Tue, 18 Apr 2023 15:00:01 GMT",
            "description": "This is the description of my JSON object",
            "link": "http://www.example.com",
            "pub_date": "Mon, 17 Apr 2023 16:08:23 GMT",
            "title": "My Json Object Title"
        }"#;        
        let spec = KeygenParams {
            lookup: vec![
                "pub_date".to_owned(), 
                "last_build_date".to_owned()
            ],
            key_name: "dedup_key".to_owned()
        };

        let record = Record::new(input);
        let result = add_key_to_json_record(&record, &spec).unwrap();

        let expected_value:Value = serde_json::from_str(expected).unwrap();
        assert_eq!(result, expected_value);

    }

}