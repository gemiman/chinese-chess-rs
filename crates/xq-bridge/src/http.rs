//! 极简 HTTP/1.1 服务（仅够本地开发用）。
//!
//! 刻意不引入 `axum` / `hyper`：这是一个跑在 `127.0.0.1` 上、服务单页面前端的
//! 开发工具，手写一个「读请求行 + 读头 + 读 body + 写响应」的循环就够，
//! 而且能避免为了一个开发工具把整套异步运行时拖进依赖树。

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;

/// 解析后的请求。
pub struct Request {
    pub method: String,
    pub path: String,
    pub body: Vec<u8>,
}

/// 待发送的响应。
pub struct Response {
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
    pub extra_headers: Vec<(String, String)>,
}

impl Response {
    pub fn json(body: String) -> Self {
        Response {
            status: 200,
            content_type: "application/json; charset=utf-8".to_string(),
            body: body.into_bytes(),
            extra_headers: Vec::new(),
        }
    }

    pub fn text(status: u16, body: String) -> Self {
        Response {
            status,
            content_type: "text/plain; charset=utf-8".to_string(),
            body: body.into_bytes(),
            extra_headers: Vec::new(),
        }
    }

    pub fn bytes(content_type: &str, body: Vec<u8>) -> Self {
        Response {
            status: 200,
            content_type: content_type.to_string(),
            body,
            extra_headers: Vec::new(),
        }
    }

    fn status_text(&self) -> &'static str {
        match self.status {
            200 => "OK",
            204 => "No Content",
            400 => "Bad Request",
            404 => "Not Found",
            405 => "Method Not Allowed",
            500 => "Internal Server Error",
            501 => "Not Implemented",
            _ => "Unknown",
        }
    }
}

/// 启动服务。`handler` 会在每个请求上被调用。
pub fn serve<F>(listener: TcpListener, handler: F) -> std::io::Result<()>
where
    F: Fn(&Request) -> Response + Send + Sync + 'static,
{
    let handler = Arc::new(handler);
    for incoming in listener.incoming() {
        let Ok(stream) = incoming else { continue };
        let handler = Arc::clone(&handler);
        // 每连接一个线程：本地单用户开发场景，连接数极少，模型足够。
        std::thread::spawn(move || {
            let _ = handle_conn(stream, handler);
        });
    }
    Ok(())
}

fn handle_conn<F>(mut stream: TcpStream, handler: Arc<F>) -> std::io::Result<()>
where
    F: Fn(&Request) -> Response,
{
    stream.set_nodelay(true).ok();

    let mut reader = BufReader::new(stream.try_clone()?);

    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(());
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let raw_path = parts.next().unwrap_or("/").to_string();
    let path = raw_path.split('?').next().unwrap_or("/").to_string();

    // 读头，取出 Content-Length
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        let lower = trimmed.to_ascii_lowercase();
        if let Some(value) = lower.strip_prefix("content-length:") {
            content_length = value.trim().parse().unwrap_or(0);
        }
    }

    // 上限保护：防止畸形请求把内存吃光
    if content_length > 8 * 1024 * 1024 {
        return write_response(&mut stream, Response::text(400, "请求体过大\n".to_string()));
    }

    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body)?;
    }

    let request = Request { method, path, body };
    let response = handler(&request);
    write_response(&mut stream, response)
}

fn write_response(stream: &mut TcpStream, response: Response) -> std::io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n",
        response.status,
        response.status_text(),
        response.content_type,
        response.body.len()
    );
    // 本地开发工具，放宽同源限制，方便 Vite 开发服务器（5173）直连本服务
    head.push_str("Access-Control-Allow-Origin: *\r\n");
    head.push_str("Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n");
    head.push_str("Access-Control-Allow-Headers: Content-Type\r\n");
    head.push_str("Cache-Control: no-store\r\n");
    for (k, v) in &response.extra_headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");

    stream.write_all(head.as_bytes())?;
    stream.write_all(&response.body)?;
    stream.flush()
}

/// 按扩展名猜测 MIME。
pub fn mime_of(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "map" => "application/json; charset=utf-8",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}
