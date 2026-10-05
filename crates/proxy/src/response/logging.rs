use std::fmt::Write;

/// 打印请求体
pub fn log_full_body(body: &str) {
    let len = body.len();
    let kb = len as f64 / 1024.0;
    tracing::info!("=== 请求体 (共 {} 字节) ===", len);
    tracing::info!("\n{}", body);
    tracing::info!("=== 请求体结束 ({} 字节 / {:.2} KB) ===", len, kb);
}

/// 打印响应体
pub fn log_full_response(body: &str) {
    let len = body.len();
    let kb = len as f64 / 1024.0;
    tracing::info!("=== 响应体 (共 {} 字节) ===", len);
    tracing::info!("{}", body);
    tracing::info!("=== 响应体结束 ({} 字节 / {:.2} KB) ===", len, kb);
}

/// 按阶段完整打印请求头，单条日志避免并发请求的头信息交错。
pub fn log_request_meta(stage: &str, method: &str, uri: &str, headers: &http::HeaderMap) {
    let mut message = format!("=== {stage}请求头 ===\nMethod: {method}\nURI: {uri}\n");

    for (name, value) in headers {
        if let Ok(value_str) = value.to_str() {
            let _ = writeln!(message, "{name}: {value_str}");
        } else {
            let _ = writeln!(message, "{name}: {value:?}");
        }
    }
    let _ = write!(message, "=== {stage}请求头结束 ===");
    tracing::info!("{message}");
}
