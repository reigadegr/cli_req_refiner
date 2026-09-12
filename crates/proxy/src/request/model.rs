use serde_json::{Value, json, to_vec};
use tracing::info;

/// 尝试覆盖请求体中的 model 字段
pub fn override_model_in_json(json: &mut Value, model: &str) {
    let original_model = json.get("model").and_then(|m| m.as_str());

    if let Some(original) = original_model {
        info!("原始 model: {} -> 覆盖为: {}", original, model);
    }

    json["model"] = json!(model);
}

const BILLING_HEADER_MARKERS: &[&str] = &[
    "x-anthropic-billing-header: cc_version",
    "You are Claude Code",
];

/// 移除 system 数组中 text 以 `BILLING_HEADER_MARKERS` 元素开头的条目，并压缩双空格为单空格
pub fn strip_billing_header_from_system(json: &mut Value) {
    let Some(system) = json.get_mut("system").and_then(|s| s.as_array_mut()) else {
        return;
    };

    // 先将每个 entry 的 text 中的双空格压缩为单空格
    for entry in system.iter_mut() {
        if let Some(text) = entry.get_mut("text").and_then(|t| t.as_str()) {
            let mut cleaned = text.to_string();
            for (pat, rep) in [
                ("  ", " "),
                ("\n\n", "\n"),
                ("\n - \n", "\n"),
                ("\n \n", "\n"),
                ("\n - ", "\n"),
            ] {
                while cleaned.contains(pat) {
                    cleaned = cleaned.replace(pat, rep);
                }
            }
            entry["text"] = json!(cleaned);
        }
    }

    system.retain(|entry| {
        entry
            .get("text")
            .and_then(|t| t.as_str())
            .is_none_or(|text| {
                !BILLING_HEADER_MARKERS
                    .iter()
                    .any(|marker| text.starts_with(marker))
            })
    });
}

pub fn override_model_in_body(body_bytes: &[u8], model: &str) -> Option<bytes::Bytes> {
    use serde_json::from_slice;
    let mut modified = from_slice::<Value>(body_bytes).ok()?;
    override_model_in_json(&mut modified, model);

    to_vec(&modified).ok().map(Into::into)
}
