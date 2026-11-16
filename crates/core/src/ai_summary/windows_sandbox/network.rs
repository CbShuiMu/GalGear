//! 沙盒通过显式继承的本地管道句柄请求 HTTP，宿主沿用系统代理。
//! 通道没有文件或命令操作，取消会丢弃在途网络 future 并回收线程。
use super::child::wide;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsStr,
    fs::File,
    io::{self, Read, Write},
    os::windows::io::{AsRawHandle, FromRawHandle},
    pin::pin,
    ptr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{ERROR_PIPE_CONNECTED, HANDLE, INVALID_HANDLE_VALUE},
    Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX},
    System::Pipes::{ConnectNamedPipe, CreateNamedPipeW, PIPE_NOWAIT, PIPE_REJECT_REMOTE_CLIENTS},
};

const MAX_BODY: usize = 64 * 1024 * 1024;
const MAX_FRAME: usize = 96 * 1024 * 1024;

pub(super) struct Broker {
    client: File,
    stopped: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

#[derive(Deserialize)]
struct Request {
    url: String,
    method: String,
    #[serde(default)]
    headers: Vec<(String, String)>,
    #[serde(default)]
    body: String,
    #[serde(default = "default_timeout")]
    timeout: u64,
    use_proxy: Option<bool>,
}
fn default_timeout() -> u64 {
    120
}

fn proxy_client() -> io::Result<reqwest::ClientBuilder> {
    let builder = reqwest::Client::builder();
    if ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"]
        .iter()
        .any(|name| std::env::var_os(name).is_some())
    {
        return Ok(builder);
    }
    use windows_sys::Win32::System::Registry::{
        HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW,
    };
    let key = wide(OsStr::new(
        r"Software\Microsoft\Windows\CurrentVersion\Internet Settings",
    ))?;
    let name = wide(OsStr::new("ProxyEnable"))?;
    let mut enabled = 0u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: fixed registry value, correct DWORD output buffer and size.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            ptr::null_mut(),
            (&mut enabled as *mut u32).cast(),
            &mut size,
        )
    };
    if status != 0 || enabled == 0 {
        return Ok(builder);
    }
    let read = |name: &str| -> io::Result<String> {
        let name = wide(OsStr::new(name))?;
        let mut size = 0;
        // SAFETY: first obtain the byte count, then read into sufficiently sized UTF-16 storage.
        unsafe {
            let status = RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_SZ,
                ptr::null_mut(),
                ptr::null_mut(),
                &mut size,
            );
            if status == 2 {
                return Ok(String::new());
            }
            if status != 0 {
                return Err(io::Error::from_raw_os_error(status as i32));
            }
            let mut buffer = vec![0u16; (size as usize).div_ceil(2)];
            let status = RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_SZ,
                ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut size,
            );
            if status != 0 {
                return Err(io::Error::from_raw_os_error(status as i32));
            }
            let end = buffer
                .iter()
                .position(|unit| *unit == 0)
                .unwrap_or(buffer.len());
            Ok(String::from_utf16_lossy(&buffer[..end]))
        }
    };
    let server = read("ProxyServer")?;
    let Some(server) = https_proxy(&server) else {
        return Ok(builder);
    };
    // WinINET 的无 scheme 地址是 HTTP CONNECT 代理，明确补齐以保持 Python urllib 的语义。
    let bypass = read("ProxyOverride")?.replace(';', ",").replace("*.", "");
    let proxy = reqwest::Proxy::https(server)
        .map_err(io::Error::other)?
        .no_proxy(reqwest::NoProxy::from_string(&bypass));
    Ok(builder.proxy(proxy))
}

fn https_proxy(server: &str) -> Option<String> {
    let server = if server.contains('=') {
        server
            .split(';')
            .find_map(|part| part.trim().strip_prefix("https="))?
    } else {
        server.trim()
    };
    if server.is_empty() {
        return None;
    }
    Some(if server.contains("://") {
        server.to_owned()
    } else {
        format!("http://{server}")
    })
}

#[derive(Serialize)]
struct Response {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Broker {
    pub(super) fn new(identity: &str) -> io::Result<Self> {
        let name = format!(r"\\.\pipe\{identity}.http");
        let pipe = create_pipe(&name)?;
        let client_pipe = File::options().read(true).write(true).open(&name)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let client = proxy_client()?
            .pool_max_idle_per_host(0)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(io::Error::other)?;
        let direct = reqwest::Client::builder()
            .no_proxy()
            .pool_max_idle_per_host(0)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(io::Error::other)?;
        let stopped = Arc::new(AtomicBool::new(false));
        let shutdown = Arc::clone(&stopped);
        let thread = thread::Builder::new()
            .name("ai-http-broker".into())
            .spawn(move || {
                use std::os::windows::io::AsRawHandle;
                let mut pipe = pipe;
                let mut pending = Vec::new();
                while !shutdown.load(Ordering::Acquire) {
                    // SAFETY: pipe is an owned, nonblocking server handle; no OVERLAPPED is used.
                    let connected =
                        unsafe { ConnectNamedPipe(pipe.as_raw_handle(), ptr::null_mut()) } != 0
                            || io::Error::last_os_error().raw_os_error()
                                == Some(ERROR_PIPE_CONNECTED as i32);
                    if !connected {
                        thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    let result = exchange(
                        &mut pipe,
                        &runtime,
                        (&client, &direct),
                        &shutdown,
                        &mut pending,
                    );
                    if let Err(error) = result {
                        // 空通道等待可以继续；损坏的帧关闭连接，让客户端收到 EOF。
                        if error.kind() != io::ErrorKind::Interrupted || !pending.is_empty() {
                            break;
                        }
                    }
                }
            })?;
        Ok(Self {
            client: client_pipe,
            stopped,
            thread: Some(thread),
        })
    }

    pub(super) fn client_handle(&self) -> HANDLE {
        self.client.as_raw_handle()
    }

    pub(super) fn stop(&mut self) -> io::Result<()> {
        self.stopped.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| io::Error::other("AI HTTP broker stopped unexpectedly"))?;
        }
        Ok(())
    }
}

impl Drop for Broker {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn exchange(
    pipe: &mut File,
    runtime: &tokio::runtime::Runtime,
    clients: (&reqwest::Client, &reqwest::Client),
    stopped: &AtomicBool,
    pending: &mut Vec<u8>,
) -> io::Result<()> {
    let input = read_frame(pipe, stopped, MAX_FRAME, pending)?;
    let request: Request = serde_json::from_slice(&input).map_err(io::Error::other)?;
    let client = if request.use_proxy.unwrap_or(true) {
        clients.0
    } else {
        clients.1
    };
    let future = fetch(client, request);
    let mut future = pin!(future);
    let result = loop {
        if stopped.load(Ordering::Acquire) {
            return Ok(());
        }
        if let Ok(result) = runtime.block_on(async {
            tokio::time::timeout(Duration::from_millis(50), future.as_mut()).await
        }) {
            break result;
        }
    };
    let mut output = match result {
        Ok(response) => serde_json::to_vec(&response),
        // 网络错误中可能有代理凭据或 URL 参数；只传回固定诊断。
        Err(_) => serde_json::to_vec(&serde_json::json!({"error":"AI HTTP request failed"})),
    }
    .map_err(io::Error::other)?;
    output.push(b'\n');
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut written = 0;
    while written < output.len() {
        check_wait(stopped, deadline)?;
        match pipe.write(&output[written..output.len().min(written + 65536)]) {
            Ok(0) => thread::sleep(Duration::from_millis(10)),
            Ok(size) => written += size,
            Err(error) if error.raw_os_error() == Some(232) => {
                thread::sleep(Duration::from_millis(10))
            }
            Err(error) => return Err(error),
        }
    }
    // 客户端确认读完响应后再接收下一条请求，保持重复请求的帧边界。
    read_frame(pipe, stopped, 8, pending)?;
    Ok(())
}

fn check_wait(stopped: &AtomicBool, deadline: Instant) -> io::Result<()> {
    if stopped.load(Ordering::Acquire) || Instant::now() > deadline {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "AI HTTP channel closed",
        ));
    }
    Ok(())
}

fn read_frame(
    pipe: &mut File,
    stopped: &AtomicBool,
    limit: usize,
    result: &mut Vec<u8>,
) -> io::Result<Vec<u8>> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut buffer = [0u8; 65536];
    let mut searched = 0;
    loop {
        check_wait(stopped, deadline)?;
        if let Some(offset) = result[searched..].iter().position(|byte| *byte == b'\n') {
            let end = searched + offset;
            if end + 1 > limit {
                return Err(io::Error::other("AI HTTP frame exceeded limit"));
            }
            let pending = result.split_off(end + 1);
            return Ok(std::mem::replace(result, pending));
        }
        if result.len() > limit {
            return Err(io::Error::other("AI HTTP frame exceeded limit"));
        }
        searched = result.len();
        match pipe.read(&mut buffer) {
            // PIPE_NOWAIT 在尚无数据时也可以返回零字节；父进程仍持有客户端句柄。
            Ok(0) => thread::sleep(Duration::from_millis(10)),
            Ok(size) => {
                result.extend_from_slice(&buffer[..size]);
            }
            Err(error) if error.raw_os_error() == Some(232) => {
                thread::sleep(Duration::from_millis(10))
            }
            Err(error) => return Err(error),
        }
    }
}

async fn fetch(
    client: &reqwest::Client,
    request: Request,
) -> Result<Response, Box<dyn std::error::Error>> {
    let url = reqwest::Url::parse(&request.url)?;
    // 转发进程具有宿主权限，仅代理现有模型端点及源码服务，避免转发到本机文件服务。
    if url.scheme() != "https"
        || !matches!(
            url.host_str(),
            Some(
                "api.openai.com"
                    | "api.deepseek.com"
                    | "api.github.com"
                    | "raw.githubusercontent.com"
            )
        )
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Unsupported AI HTTP URL".into());
    }
    let body = STANDARD.decode(request.body)?;
    if body.len() > MAX_BODY {
        return Err("AI HTTP body exceeded limit".into());
    }
    let mut builder = client
        .request(reqwest::Method::from_bytes(request.method.as_bytes())?, url)
        .timeout(Duration::from_secs(request.timeout.clamp(1, 180)))
        .header(reqwest::header::CONNECTION, "close")
        .body(body);
    for (name, value) in request.headers {
        if ![
            "host",
            "connection",
            "content-length",
            "proxy-authorization",
        ]
        .contains(&name.to_ascii_lowercase().as_str())
        {
            builder = builder.header(name, value);
        }
    }
    let mut response = builder.send().await?;
    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .map(|(key, value)| Ok((key.to_string(), value.to_str()?.to_owned())))
        .collect::<Result<Vec<_>, reqwest::header::ToStrError>>()?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len() + chunk.len() > MAX_BODY {
            return Err("AI HTTP response exceeded limit".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(Response {
        status,
        headers,
        body: STANDARD.encode(bytes),
    })
}

fn create_pipe(name: &str) -> io::Result<File> {
    let name = wide(name.as_ref())?;
    // SAFETY: this is a fresh local non-inheritable pipe. The client is opened by the parent;
    // only that handle is passed to this task, so no filesystem/namespace permission is added.
    let handle = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            65536,
            65536,
            0,
            ptr::null(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: CreateNamedPipeW returned a uniquely owned handle.
    Ok(unsafe { File::from_raw_handle(handle) })
}

#[cfg(test)]
mod tests {
    use super::{Broker, https_proxy};
    use std::io::{BufRead, BufReader, Write};

    #[test]
    fn windows_proxy_addresses_keep_http_connect_and_https_selection() {
        assert_eq!(
            https_proxy("127.0.0.1:7890").as_deref(),
            Some("http://127.0.0.1:7890")
        );
        assert_eq!(
            https_proxy("http=127.0.0.1:80;https=127.0.0.1:443").as_deref(),
            Some("http://127.0.0.1:443")
        );
        assert_eq!(
            https_proxy("https://proxy.example:443").as_deref(),
            Some("https://proxy.example:443")
        );
        assert!(https_proxy("http=127.0.0.1:80").is_none());
    }

    #[test]
    fn broker_rejects_file_and_local_service_requests_and_keeps_frame_boundaries() {
        let temporary = tempfile::tempdir().unwrap();
        let identity = format!(
            "GalGear.NetworkTest.{}",
            temporary.path().file_name().unwrap().to_string_lossy()
        );
        let mut broker = Broker::new(&identity).unwrap();
        let mut client = broker.client.try_clone().unwrap();
        writeln!(
            client,
            "{}",
            serde_json::json!({"url":"file:///C:/private", "method":"GET"})
        )
        .unwrap();
        let mut response = String::new();
        BufReader::new(&mut client)
            .read_line(&mut response)
            .unwrap();
        assert!(serde_json::from_str::<serde_json::Value>(&response).unwrap()["error"].is_string());
        // 同一次写入包含确认和下一条请求，服务端必须保留第一行后的字节。
        writeln!(
            client,
            "ok\n{}",
            serde_json::json!({"url":"https://127.0.0.1/private", "method":"GET"})
        )
        .unwrap();
        response.clear();
        BufReader::new(&mut client)
            .read_line(&mut response)
            .unwrap();
        assert!(serde_json::from_str::<serde_json::Value>(&response).unwrap()["error"].is_string());
        // 客户端尚未确认也可以取消，不能等待管道超时。
        let started = std::time::Instant::now();
        broker.stop().unwrap();
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
    }
}
