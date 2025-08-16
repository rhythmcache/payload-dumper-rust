use crate::module::utils::format_size;
use anyhow::{Result, anyhow};
use lazy_static::lazy_static;
use reqwest::{
    blocking::{Client, Response},
    header,
};
use std::io::{self, Read, Seek, SeekFrom};
#[cfg(feature = "hickory-dns")]
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use url;

#[cfg(feature = "hickory-dns")]
use once_cell::sync::Lazy;

#[cfg(feature = "hickory-dns")]
use reqwest_hickory_resolver::HickoryResolver;

#[cfg(feature = "hickory-dns")]
static GLOBAL_RESOLVER: Lazy<Arc<HickoryResolver>> =
    Lazy::new(|| Arc::new(HickoryResolver::default()));

const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

lazy_static! {
    static ref ACCEPT_RANGES_WARNING_SHOWN: AtomicBool = AtomicBool::new(false);
    static ref FILE_SIZE_INFO_SHOWN: AtomicBool = AtomicBool::new(false);
}

// Function to create HTTP client
pub fn create_http_client(user_agent: Option<&str>) -> Result<Client> {
    let mut headers = header::HeaderMap::new();
    headers.insert(
        header::ACCEPT_ENCODING,
        header::HeaderValue::from_static("gzip, deflate, br"),
    );
    headers.insert(header::ACCEPT, header::HeaderValue::from_static("*/*"));

    // use custom user agent if provided otherwise use default
    let ua = user_agent.unwrap_or(DEFAULT_USER_AGENT);
    headers.insert(
        header::USER_AGENT,
        header::HeaderValue::from_str(ua)
            .map_err(|e| anyhow!("Invalid user agent string: {}", e))?,
    );

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

    client_builder
        .build()
        .map_err(|e| anyhow!("Failed to create HTTP client: {}", e))
}

pub struct HttpReader {
    url: String,
    position: u64,
    pub content_length: u64,
    client: Client,
    pub content_type: Option<String>,
    supports_ranges: bool,
}

impl Clone for HttpReader {
    fn clone(&self) -> Self {
        Self {
            url: self.url.clone(),
            position: self.position,
            content_length: self.content_length,
            client: self.client.clone(),
            content_type: self.content_type.clone(),
            supports_ranges: self.supports_ranges,
        }
    }
}

impl HttpReader {
    // pub fn new(url: String) -> Result<Self> {
    //    Self::new_with_user_agent(url, None, true)
    //   }

    pub fn new_silent(url: String) -> Result<Self> {
        Self::new_with_user_agent(url, None, false)
    }

    pub fn new_with_user_agent(
        url: String,
        user_agent: Option<&str>,
        print_size: bool,
    ) -> Result<Self> {
        let client = create_http_client(user_agent)?;

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

                    // Check if server supports range requests
                    let supports_ranges = response
                        .headers()
                        .get(header::ACCEPT_RANGES)
                        .and_then(|v| v.to_str().ok())
                        .map(|v| v == "bytes")
                        .unwrap_or(false);

                    if !supports_ranges && !ACCEPT_RANGES_WARNING_SHOWN.swap(true, Ordering::SeqCst)
                    {
                        eprintln!(
                            "- Warning: Server doesn't advertise Accept-Ranges. The process may fail."
                        );
                    }

                    // Print file size info if requested
                    if print_size && !FILE_SIZE_INFO_SHOWN.swap(true, Ordering::SeqCst) {
                        eprintln!("- File size: {}", format_size(content_length));
                    }

                    return Ok(Self {
                        url,
                        position: 0,
                        content_length,
                        client,
                        content_type,
                        supports_ranges,
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

    pub fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        if offset >= self.content_length {
            return Ok(0);
        }

        let remaining = self.content_length - offset;
        let to_read = std::cmp::min(buf.len() as u64, remaining) as usize;

        if to_read == 0 {
            return Ok(0);
        }

        let end = offset + to_read as u64 - 1;
        let range = format!("bytes={}-{}", offset, end);

        let mut retry_count = 0;
        let max_retries = 3;

        while retry_count < max_retries {
            match self
                .client
                .get(&self.url)
                .header(header::RANGE, range.clone())
                .send()
            {
                Ok(mut response) => {
                    if !response.status().is_success() && response.status().as_u16() != 206 {
                        return Err(io::Error::other(format!(
                            "HTTP error: {} for range {}",
                            response.status(),
                            range
                        )));
                    }

                    return copy_from_response(&mut response, &mut buf[..to_read]);
                }
                Err(e) => {
                    retry_count += 1;
                    if retry_count == max_retries {
                        return Err(io::Error::other(format!(
                            "Failed to read range {} after {} retries: {}",
                            range, max_retries, e
                        )));
                    }
                    std::thread::sleep(Duration::from_secs(2 * retry_count as u64));
                }
            }
        }

        Err(io::Error::other(
            "Failed to read range after maximum retries",
        ))
    }
}

impl Read for HttpReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let bytes_read = self.read_at(self.position, buf)?;
        self.position += bytes_read as u64;
        Ok(bytes_read)
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
                    self.position.saturating_sub(offset.unsigned_abs())
                }
            }
            SeekFrom::End(offset) => {
                if offset >= 0 {
                    self.content_length.saturating_add(offset as u64)
                } else {
                    self.content_length.saturating_sub(offset.unsigned_abs())
                }
            }
        };

        if new_pos > self.content_length {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "Seek position {} exceeds content length {}",
                    new_pos, self.content_length
                ),
            ));
        }

        self.position = new_pos;
        Ok(self.position)
    }
}

pub fn copy_from_response(response: &mut Response, buf: &mut [u8]) -> io::Result<usize> {
    let mut total_read = 0;

    while total_read < buf.len() {
        match response.read(&mut buf[total_read..]) {
            Ok(0) => break, // EOF
            Ok(n) => total_read += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }

    Ok(total_read)
}
