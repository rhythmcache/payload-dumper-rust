// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2025 rhythmcache
// https://github.com/rhythmcache/payload-dumper-rust

#![allow(unused)]
use crate::constants::DEFAULT_USER_AGENT;
use anyhow::{Result, anyhow};
use reqwest::{Client, header};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

// global DNS resolver for hickory_dns feature
#[cfg(feature = "hickory_dns")]
use std::sync::{Arc, OnceLock};

#[cfg(feature = "hickory_dns")]
static GLOBAL_DNS_RESOLVER: OnceLock<
    Arc<hickory_resolver::Resolver<hickory_resolver::name_server::TokioConnectionProvider>>,
> = OnceLock::new();

#[cfg(feature = "hickory_dns")]
async fn get_or_init_dns_resolver()
-> Result<Arc<hickory_resolver::Resolver<hickory_resolver::name_server::TokioConnectionProvider>>> {
    use hickory_proto::xfer::Protocol;
    use hickory_resolver::Resolver;
    use hickory_resolver::config::*;
    use hickory_resolver::name_server::TokioConnectionProvider;

    if let Some(resolver) = GLOBAL_DNS_RESOLVER.get() {
        return Ok(resolver.clone());
    }

    // check for custom DNS from environment variable
    let config = if let Ok(custom_dns) = std::env::var("PAYLOAD_DUMPER_CUSTOM_DNS") {
        // parse custom DNS servers (comma-separated, for example., "8.8.8.8,8.8.4.4")
        let dns_ips: Result<Vec<_>> = custom_dns
            .split(',')
            .map(|s| s.trim().parse::<std::net::IpAddr>())
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| anyhow!("Invalid DNS IP in PAYLOAD_DUMPER_CUSTOM_DNS: {}", e));

        let dns_ips = dns_ips?;

        if dns_ips.is_empty() {
            return Err(anyhow!("PAYLOAD_DUMPER_CUSTOM_DNS is empty"));
        }

        // Create config with custom DNS servers
        let mut config = ResolverConfig::new();
        for ip in dns_ips {
            let socket_addr = std::net::SocketAddr::new(ip, 53);
            config.add_name_server(NameServerConfig {
                socket_addr,
                protocol: Protocol::Udp,
                tls_dns_name: None,
                http_endpoint: None,
                trust_negative_responses: true,
                bind_addr: None,
            });
        }
        config
    } else {
        // use Cloudflare DNS by default
        ResolverConfig::cloudflare()
    };

    // build resolver in spawn_blocking to avoid blocking async runtime
    let resolver = tokio::task::spawn_blocking(move || {
        Resolver::builder_with_config(config, TokioConnectionProvider::default()).build()
    })
    .await
    .map_err(|e| anyhow!("Failed to spawn resolver task: {}", e))?;

    let resolver = Arc::new(resolver);

    // try to initialize, but use existing if another task beat us to it
    Ok(GLOBAL_DNS_RESOLVER.get_or_init(|| resolver.clone()).clone())
}

/// HTTP client
async fn create_http_client(user_agent: Option<&str>) -> Result<Client> {
    let mut headers = header::HeaderMap::new();

    let ua = user_agent.unwrap_or(DEFAULT_USER_AGENT);
    headers.insert(
        header::USER_AGENT,
        header::HeaderValue::from_str(ua)
            .map_err(|e| anyhow!("Invalid user agent string: {}", e))?,
    );

    headers.insert(
        header::ACCEPT_ENCODING,
        header::HeaderValue::from_static("gzip, deflate, br"),
    );
    headers.insert(header::ACCEPT, header::HeaderValue::from_static("*/*"));
    headers.insert(
        header::CONNECTION,
        header::HeaderValue::from_static("keep-alive"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-transform"),
    );

    let mut client_builder = Client::builder()
        .timeout(Duration::from_secs(600))
        .connect_timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(10)
        .pool_idle_timeout(Duration::from_secs(90))
        .tcp_keepalive(Some(Duration::from_secs(30)))
        .http2_keep_alive_interval(Some(Duration::from_secs(30)))
        .http2_adaptive_window(true)
        .default_headers(headers)
        .redirect(reqwest::redirect::Policy::limited(10));

    // use custom DNS resolver when feature is enabled
    #[cfg(feature = "hickory_dns")]
    {
        use hickory_resolver::name_server::TokioConnectionProvider;
        use reqwest::dns::{Name, Resolve, Resolving};
        use std::net::SocketAddr;

        struct CustomDnsResolver {
            resolver: Arc<hickory_resolver::Resolver<TokioConnectionProvider>>,
        }

        impl Resolve for CustomDnsResolver {
            fn resolve(&self, name: Name) -> Resolving {
                let resolver = self.resolver.clone();
                Box::pin(async move {
                    let name_str = name.as_str();
                    let lookup = resolver
                        .lookup_ip(name_str)
                        .await
                        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

                    let addrs: Box<dyn Iterator<Item = SocketAddr> + Send> =
                        Box::new(lookup.into_iter().map(|ip| SocketAddr::new(ip, 0)));

                    Ok(addrs)
                })
            }
        }

        let resolver = get_or_init_dns_resolver()
            .await
            .map_err(|e| anyhow!("Failed to create DNS resolver: {}", e))?;

        client_builder = client_builder.dns_resolver(Arc::new(CustomDnsResolver { resolver }));
    }

    client_builder
        .build()
        .map_err(|e| anyhow!("Failed to create HTTP client: {}", e))
}

/// async HTTP reader with range request support
pub struct HttpReader {
    pub client: Client,
    pub url: String,
    pub content_length: u64,
}

impl HttpReader {
    pub async fn new(url: String, user_agent: Option<&str>) -> Result<Self> {
        let client = create_http_client(user_agent).await?;

        // validate URL
        url::Url::parse(&url).map_err(|e| anyhow!("Invalid URL: {}", e))?;

        // head request with retries
        let mut retry_count = 0;
        const MAX_RETRIES: u32 = 3;
        let mut last_error = None;

        while retry_count < MAX_RETRIES {
            match client.head(&url).send().await {
                Ok(response) => {
                    if !response.status().is_success() {
                        return Err(anyhow!("Failed to access URL: {}", response.status()));
                    }

                    // check range support
                    let supports_ranges = response
                        .headers()
                        .get(header::ACCEPT_RANGES)
                        .and_then(|v| v.to_str().ok())
                        .map(|v| v == "bytes")
                        .unwrap_or(false);
                    /*
                                        if !supports_ranges {
                                            ACCEPT_RANGES_WARNING_SHOWN.get_or_init(|| {
                                                eprintln!("- Warning: Server doesn't advertise Accept-Ranges: bytes");
                                                eprintln!(
                                                    "- Extraction may fail if server doesn't support range requests"
                                                );
                                            });
                                        }
                    */
                    // get content length
                    let content_length = response
                        .headers()
                        .get(header::CONTENT_LENGTH)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.parse::<u64>().ok())
                        .ok_or_else(|| anyhow!("Could not determine content length"))?;

                    if content_length == 0 {
                        return Err(anyhow!("File size is 0"));
                    }

                    return Ok(Self {
                        client,
                        url,
                        content_length,
                    });
                }
                Err(e) => {
                    last_error = Some(e);
                    retry_count += 1;
                    if retry_count < MAX_RETRIES {
                        tokio::time::sleep(Duration::from_secs(2u64.pow(retry_count))).await;
                    }
                }
            }
        }

        Err(anyhow!(
            "Failed to connect after {} retries. Last error: {}",
            MAX_RETRIES,
            last_error.unwrap()
        ))
    }

    /// read exact bytes at specific offset
    pub async fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
        if offset >= self.content_length {
            return Err(anyhow!(
                "Offset {} exceeds content length {}",
                offset,
                self.content_length
            ));
        }

        // clamp the read to available bytes
        let remaining = self.content_length - offset;
        let to_read = std::cmp::min(buf.len() as u64, remaining) as usize;

        if to_read == 0 {
            return Ok(());
        }

        // calculate inclusive end for range header
        let end = offset + to_read as u64 - 1;
        let range_header = format!("bytes={}-{}", offset, end);

        let mut retry_count = 0;
        const MAX_RETRIES: u32 = 3;
        let mut last_error = None;

        while retry_count < MAX_RETRIES {
            match self
                .client
                .get(&self.url)
                .header(header::RANGE, &range_header)
                .send()
                .await
            {
                Ok(response) => {
                    let status = response.status();
                    if !status.is_success() && status.as_u16() != 206 {
                        return Err(anyhow!("Range request failed: {}", status));
                    }

                    let bytes = response.bytes().await?;

                    if bytes.len() != to_read {
                        return Err(anyhow!(
                            "Server returned incorrect bytes: expected {}, got {}",
                            to_read,
                            bytes.len()
                        ));
                    }

                    buf[..to_read].copy_from_slice(&bytes);
                    return Ok(());
                }
                Err(e) => {
                    last_error = Some(e);
                    retry_count += 1;
                    if retry_count < MAX_RETRIES {
                        tokio::time::sleep(Duration::from_secs(2u64.pow(retry_count))).await;
                    }
                }
            }
        }

        Err(anyhow!(
            "Failed to read after {} retries. Last error: {}",
            MAX_RETRIES,
            last_error.unwrap()
        ))
    }
}

// zipIO trait for HttpReader so it can be used with ZipParser
#[async_trait::async_trait]
impl crate::zip::zip_io::ZipIO for HttpReader {
    async fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
        self.read_at(offset, buf).await
    }

    async fn size(&self) -> Result<u64> {
        Ok(self.content_length)
    }
}
