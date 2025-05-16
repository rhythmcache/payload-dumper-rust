use anyhow::{Result, anyhow};
use lazy_static::lazy_static;
use reqwest::{
    blocking::{Client, Response},
    header,
};
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use url;

#[cfg(feature = "hickory-dns")]
use std::sync::Arc;

#[cfg(feature = "hickory-dns")]
use once_cell::sync::Lazy;

#[cfg(feature = "hickory-dns")]
use reqwest_hickory_resolver::HickoryResolver;

#[cfg(feature = "hickory-dns")]
static GLOBAL_RESOLVER: Lazy<Arc<HickoryResolver>> =
    Lazy::new(|| Arc::new(HickoryResolver::default()));

lazy_static! {
    static ref HTTP_CLIENT: Client = {
        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::ACCEPT_ENCODING,
            header::HeaderValue::from_static("gzip, deflate, br"),
        );
        headers.insert(header::ACCEPT, header::HeaderValue::from_static("*/*"));
        headers.insert(header::USER_AGENT, header::HeaderValue::from_static(
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36"
        ));
        headers.insert(
            header::ACCEPT_RANGES,
            header::HeaderValue::from_static("bytes"),
        );
        headers.insert(
            header::CONNECTION,
            header::HeaderValue::from_static("keep-alive"),
        );
        headers.insert(
            header::CACHE_CONTROL,
            header::HeaderValue::from_static("no-transform"),
        );

        let client_builder = Client::builder()
            .timeout(Duration::from_secs(600))
            .tcp_keepalive(Some(Duration::from_secs(30)))
            .pool_max_idle_per_host(10)
            .default_headers(headers)
            .redirect(reqwest::redirect::Policy::limited(10));

        #[cfg(feature = "hickory-dns")]
        let client_builder = client_builder.dns_resolver(GLOBAL_RESOLVER.clone());

        client_builder.build().unwrap_or_else(|_| Client::new())
    };
    static ref ACCEPT_RANGES_WARNING_SHOWN: AtomicBool = AtomicBool::new(false);
    static ref FILE_SIZE_INFO_SHOWN: AtomicBool = AtomicBool::new(false);
}

pub struct HttpReader {
    url: String,
    position: u64,
    pub content_length: u64,
    client: Client,
    pub content_type: Option<String>,
}

impl Clone for HttpReader {
    fn clone(&self) -> Self {
        Self {
            url: self.url.clone(),
            position: self.position,
            content_length: self.content_length,
            client: HTTP_CLIENT.clone(),
            content_type: self.content_type.clone(),
        }
    }
}

impl HttpReader {
    pub fn new(url: String) -> Result<Self> {
        Self::new_internal(url, true)
    }

    pub fn new_silent(url: String) -> Result<Self> {
        Self::new_internal(url, false)
    }

    fn new_internal(url: String, print_size: bool) -> Result<Self> {
        let client = HTTP_CLIENT.clone();

        let parsed_url = url::Url::parse(&url).map_err(|e| anyhow!("Invalid URL: {}", e))?;

        let _host = parsed_url
            .host_str()
            .ok_or_else(|| anyhow!("No host in URL"))?;
        let _port = parsed_url
            .port()
            .unwrap_or(if parsed_url.scheme() == "https" {
                443
            } else {
                80
            });

        let mut retry_count = 0;
        let max_retries = 3;
        let mut last_error = None;

        while retry_count < max_retries {
            match client.head(&url).send() {
                Ok(response) => {
                    let content_type = response
                        .headers()
                        .get(header::CONTENT_TYPE)
                        .and_then(|v| v.to_str().ok())
                        .map(|s| s.to_string());

                    let content_length = response
                        .headers()
                        .get(header::CONTENT_LENGTH)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.parse::<u64>().ok())
                        .ok_or_else(|| anyhow!("Could not determine content length"))?;

                    // Check if server supports Accept-Ranges header
                    if !response.headers().contains_key(header::ACCEPT_RANGES) {
                        if !ACCEPT_RANGES_WARNING_SHOWN.swap(true, Ordering::SeqCst) {
                            eprintln!(
                                "- Warning: Server doesn't advertise Accept-Ranges. The process may fail."
                            );
                        }
                    }
                    
                    // Print file size info if requested
                    if print_size && !FILE_SIZE_INFO_SHOWN.swap(true, Ordering::SeqCst) {
                        let size_mb = content_length as f64 / (1024.0 * 1024.0);
                        eprintln!("- File size: {:.2} MB", size_mb);
                    }
                    
                    return Ok(Self {
                        url,
                        position: 0,
                        content_length,
                        client,
                        content_type,
                    });
                }
                Err(e) => {
                    last_error = Some(e);
                    retry_count += 1;
                    if retry_count < max_retries {
                        std::thread::sleep(Duration::from_secs(2 * retry_count as u64));
                    }
                }
            }
        }

        Err(anyhow!(
            "Failed to connect after {} retries. Last error: {}",
            max_retries,
            last_error.unwrap()
        ))
    }

    fn read_range(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.position >= self.content_length {
            return Ok(0);
        }

        let start = self.position;
        let end = std::cmp::min(start + buf.len() as u64 - 1, self.content_length - 1);
        let to_read = (end - start + 1) as usize;

        if to_read == 0 {
            return Ok(0);
        }

        let chunk_size = std::cmp::min(to_read, 4 * 1024 * 1024);
        let range = format!("bytes={}-{}", start, start + chunk_size as u64 - 1);

        let mut retry_count = 0;
        let max_retries = 3;

        while retry_count < max_retries {
            let request = self
                .client
                .get(&self.url)
                .header(header::RANGE, range.clone())
                .header(header::CONNECTION, "keep-alive");

            match request.send() {
                Ok(mut response) => {
                    if !response.status().is_success() {
                        return Err(io::Error::new(
                            io::ErrorKind::Other,
                            format!("Failed to access URL range: {}", response.status()),
                        ));
                    }

                    let mut bytes_read = 0;
                    while bytes_read < to_read {
                        match copy_from_response(&mut response, &mut buf[bytes_read..to_read]) {
                            Ok(0) => break,
                            Ok(n) => bytes_read += n,
                            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                            Err(e) => return Err(e),
                        }
                    }

                    self.position += bytes_read as u64;
                    return Ok(bytes_read);
                }
                Err(e) => {
                    retry_count += 1;
                    if retry_count == max_retries {
                        return Err(io::Error::new(io::ErrorKind::Other, e.to_string()));
                    }
                    std::thread::sleep(Duration::from_secs(2 * retry_count as u64));
                }
            }
        }

        Err(io::Error::new(
            io::ErrorKind::Other,
            "Failed to read range after maximum retries",
        ))
    }
}

impl Read for HttpReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.read_range(buf)
    }
}

impl Seek for HttpReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let new_pos = match pos {
            SeekFrom::Start(offset) => offset,
            SeekFrom::Current(offset) => {
                if offset >= 0 {
                    self.position.saturating_add(offset as u64)
                } else {
                    self.position.saturating_sub(offset.abs() as u64)
                }
            }
            SeekFrom::End(offset) => {
                if offset >= 0 {
                    self.content_length.saturating_add(offset as u64)
                } else {
                    self.content_length.saturating_sub(offset.abs() as u64)
                }
            }
        };
        if new_pos > self.content_length {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Attempted to seek past end of file",
            ));
        }
        self.position = new_pos;
        Ok(self.position)
    }
}

pub fn copy_from_response(response: &mut Response, buf: &mut [u8]) -> io::Result<usize> {
    use std::io::Read;
    let mut reader = response.by_ref().take(buf.len() as u64);
    let mut bytes_read = 0;

    while bytes_read < buf.len() {
        match reader.read(&mut buf[bytes_read..]) {
            Ok(0) => break,
            Ok(n) => bytes_read += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }

    Ok(bytes_read)
}
