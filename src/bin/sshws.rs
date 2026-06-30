use std::net::SocketAddr;
use std::sync::Arc;

use tokio::io::{self, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use clap::Parser;

// ── Constants ────────────────────────────────────────────────────────────────

const BUFLEN: usize = 4096 * 4;
const SOCKS_VERSION: u8 = 5;

/// Sent to the client after a successful CONNECT tunnel is established.
const RESPONSE: &str = concat!(
    "HTTP/1.1 101 <b><i><font color=\"blue\">RYOTWELL.VERCEL.APP</font></b> Switching Protocols\r\n",
    "Upgrade: websocket\r\n",
    "Connection: Upgrade\r\n",
    "Sec-WebSocket-Accept: foo\r\n",
    "\r\n"
);

// ── CLI ──────────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "sshws",
    about = "SSH-free HTTP CONNECT / SOCKS5 proxy.\n\
             Set --pass to authenticate clients; traffic is forwarded\n\
             directly to whatever host:port the client requests."
)]
struct Args {
    /// Bind address
    #[arg(short = 'b', long = "bind", default_value = "0.0.0.0")]
    bind: String,

    /// Listening port
    #[arg(short = 'p', long = "port", default_value_t = 700)]
    port: u16,

    /// Shared password — clients must supply this in the X-Pass header.
    /// If omitted, only requests targeting 127.0.0.1 / localhost are allowed.
    #[arg(long = "pass", default_value = "")]
    pass: String,
}

// ── Server ───────────────────────────────────────────────────────────────────

struct Server {
    addr: String,
    port: u16,
    pass: Arc<String>,
}

impl Server {
    fn new(addr: &str, port: u16, pass: &str) -> Self {
        Self {
            addr: addr.to_owned(),
            port,
            pass: Arc::new(pass.to_owned()),
        }
    }

    async fn run(&self) -> io::Result<()> {
        let bind_addr = format!("{}:{}", self.addr, self.port);
        let listener = TcpListener::bind(&bind_addr).await?;

        println!("\n:-------RustProxy (SSH-free)-------:");
        println!("  Bind  : {}", bind_addr);
        if self.pass.is_empty() {
            println!("  Auth  : none (local-only mode)");
        } else {
            println!("  Auth  : X-Pass header");
        }
        println!(":----------------------------------:\n");

        loop {
            match listener.accept().await {
                Ok((socket, addr)) => {
                    let pass = Arc::clone(&self.pass);
                    tokio::spawn(async move {
                        handle_connection(socket, addr, pass).await;
                    });
                }
                Err(e) => eprintln!("Accept error: {e}"),
            }
        }
    }
}

// ── Connection handler ───────────────────────────────────────────────────────

async fn handle_connection(mut client: TcpStream, addr: SocketAddr, pass: Arc<String>) {
    let mut log = format!("[{addr}]");
    let mut buf = vec![0u8; BUFLEN];

    let n = match client.read(&mut buf).await {
        Ok(0) | Err(_) => return,
        Ok(n) => n,
    };
    let initial = buf[..n].to_vec();

    let result = if initial[0] == SOCKS_VERSION {
        handle_socks5(&mut client, &mut log, initial).await
    } else {
        handle_http(&mut client, &mut log, initial, &pass).await
    };

    if let Err(e) = result {
        eprintln!("{log} error: {e}");
    }
}

// ── HTTP CONNECT handler ─────────────────────────────────────────────────────

async fn handle_http(
    client: &mut TcpStream,
    log: &mut String,
    initial: Vec<u8>,
    pass: &str,
) -> io::Result<()> {
    let headers = String::from_utf8_lossy(&initial).into_owned();

    // X-Real-Host: target destination (host:port).
    // Falls back to the first CONNECT / GET host line if absent.
    let host_port = find_header(&headers, "X-Real-Host")
        .filter(|s| !s.is_empty())
        .or_else(|| extract_connect_host(&headers))
        .unwrap_or_else(|| "127.0.0.1:80".to_owned());

    // X-Split: swallow an extra buffer before proxying (anti-DPI padding).
    if find_header(&headers, "X-Split").map_or(false, |v| !v.is_empty()) {
        let mut tmp = vec![0u8; BUFLEN];
        let _ = client.read(&mut tmp).await;
    }

    // ── Auth ──────────────────────────────────────────────────────────────
    // If --pass was set, the client must supply a matching X-Pass header.
    // If --pass is empty, only loopback destinations are allowed (safe default).
    if !pass.is_empty() {
        let supplied = find_header(&headers, "X-Pass").unwrap_or_default();
        if supplied != pass {
            client.write_all(b"HTTP/1.1 407 Proxy Auth Required\r\n\r\n").await?;
            return Ok(());
        }
        // Password correct — forward to whatever the client requested.
        method_connect(client, log, &host_port).await
    } else if host_port.starts_with("127.0.0.1") || host_port.starts_with("localhost") {
        // No password configured — only allow local destinations.
        method_connect(client, log, &host_port).await
    } else {
        client.write_all(b"HTTP/1.1 403 Forbidden\r\n\r\n").await?;
        Ok(())
    }
}

async fn method_connect(
    client: &mut TcpStream,
    log: &mut String,
    path: &str,
) -> io::Result<()> {
    log.push_str(&format!(" CONNECT {path}"));

    let mut target = match connect_target(path).await {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{log} → target unreachable: {e}");
            client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await?;
            return Ok(());
        }
    };

    client.write_all(RESPONSE.as_bytes()).await?;
    println!("{log}");
    
    do_connect(client, &mut target).await
}

// ── SOCKS5 handler ───────────────────────────────────────────────────────────

async fn handle_socks5(
    client: &mut TcpStream,
    log: &mut String,
    _initial: Vec<u8>,
) -> io::Result<()> {
    // No-auth handshake reply
    client.write_all(&[0x05, 0x00]).await?;

    // Read request header: VER CMD RSV ATYP
    let mut req_hdr = [0u8; 4];
    client.read_exact(&mut req_hdr).await?;
    let (_ver, cmd, _rsv, atyp) = (req_hdr[0], req_hdr[1], req_hdr[2], req_hdr[3]);

    if cmd != 1 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Only SOCKS5 CONNECT (0x01) is supported",
        ));
    }

    let address = match atyp {
        1 => {
            let mut ipv4 = [0u8; 4];
            client.read_exact(&mut ipv4).await?;
            format!("{}.{}.{}.{}", ipv4[0], ipv4[1], ipv4[2], ipv4[3])
        }
        3 => {
            let mut len = [0u8; 1];
            client.read_exact(&mut len).await?;
            let mut domain = vec![0u8; len[0] as usize];
            client.read_exact(&mut domain).await?;
            String::from_utf8_lossy(&domain).into_owned()
        }
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!("Unsupported SOCKS5 address type: {atyp}"),
            ));
        }
    };

    let mut port_buf = [0u8; 2];
    client.read_exact(&mut port_buf).await?;
    let port = u16::from_be_bytes(port_buf);

    log.push_str(&format!(" SOCKS5 {address}:{port}"));

    let target_addr = format!("{address}:{port}");
    let mut target = match connect_target(&target_addr).await {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{log} → target unreachable: {e}");
            return Err(e);
        }
    };

    // Success reply
    client
        .write_all(&[0x05, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00])
        .await?;

    println!("{log}");
    do_connect(client, &mut target).await
}

// ── Bidirectional tunnel ─────────────────────────────────────────────────────

// ponytail: tokio covers bidirectional copy. Skipped custom idle timeout, add when TCP keepalives aren't enough.
async fn do_connect(client: &mut TcpStream, target: &mut TcpStream) -> io::Result<()> {
    tokio::io::copy_bidirectional(client, target).await.map(|_| ())
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Connect to `host:port` (DNS resolved by Tokio).
async fn connect_target(host: &str) -> io::Result<TcpStream> {
    let (h, p) = if let Some(pos) = host.rfind(':') {
        let port: u16 = host[pos + 1..].parse().unwrap_or(443);
        (&host[..pos], port)
    } else {
        (host, 443u16)
    };
    TcpStream::connect(format!("{h}:{p}")).await
}

/// Extract the host:port from a `CONNECT host:port HTTP/1.1` request line.
fn extract_connect_host(headers: &str) -> Option<String> {
    let first_line = headers.split("\r\n").next()?;
    // "CONNECT example.com:443 HTTP/1.1"
    let mut parts = first_line.split_whitespace();
    let method = parts.next()?;
    if method.eq_ignore_ascii_case("CONNECT") {
        parts.next().map(str::to_owned)
    } else {
        None
    }
}

/// Return the value of an HTTP header line (case-sensitive name match).
fn find_header(headers: &str, name: &str) -> Option<String> {
    let prefix = format!("{name}: ");
    for line in headers.split("\r\n") {
        if let Some(val) = line.strip_prefix(&prefix) {
            return Some(val.to_owned());
        }
    }
    None
}

// ── Entry point ──────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let args = Args::parse();
    let server = Server::new(&args.bind, args.port, &args.pass);
    if let Err(e) = server.run().await {
        eprintln!("Fatal server error: {e}");
        std::process::exit(1);
    }
}