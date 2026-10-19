use std::path::PathBuf;

pub use reqwest::header::HeaderValue;

/// Controls how [`crate::HttpTiles`] use the HTTP protocol, such as caching.
pub struct HttpOptions {
    /// Path to the directory to store the HTTP cache.
    ///
    /// The cache is a single redb database (`http-cache.redb`) held under an exclusive lock, so
    /// only one client at a time can use a given directory; others run without a cache.
    ///
    /// Keep in mind that some providers (such as OpenStreetMap) require clients
    /// to respect the HTTP `Expires` header.
    /// <https://operations.osmfoundation.org/policies/tiles/>
    ///
    /// This option is ignored in WASM, as HTTP cache is controlled by the
    /// browser the app is running on.
    pub cache: Option<PathBuf>,

    /// User agent to be sent to the tile servers.
    ///
    /// This should be set only on native targets. The browser sets its own user agent on wasm
    /// targets, and trying to set a different one may upset some servers (e.g. MapBox)
    pub user_agent: Option<HeaderValue>,

    /// Maximum number of parallel downloads.
    ///
    /// Many services have rate limits, and exceeding them may result in throttling, bans, or
    /// degraded service. Use the default value when in doubt.
    pub max_parallel_downloads: MaxParallelDownloads,

    /// TLS configuration, such as the trusted roots. By default, certificates are verified with
    /// `rustls-platform-verifier`, which on Android must be initialized before the first tile is
    /// fetched.
    #[cfg(not(target_arch = "wasm32"))]
    pub tls: Option<rustls::ClientConfig>,
}

impl Default for HttpOptions {
    fn default() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        let user_agent = Some(HeaderValue::from_static(concat!(
            env!("CARGO_PKG_NAME"),
            "/",
            env!("CARGO_PKG_VERSION"),
        )));

        #[cfg(target_arch = "wasm32")]
        let user_agent = None;

        Self {
            cache: None,
            user_agent,
            max_parallel_downloads: MaxParallelDownloads::default(),
            #[cfg(not(target_arch = "wasm32"))]
            tls: None,
        }
    }
}

/// Maximum number of parallel downloads.
pub struct MaxParallelDownloads(pub usize);

impl Default for MaxParallelDownloads {
    /// Default number of parallel downloads. Following modern browsers' behavior.
    /// <https://stackoverflow.com/questions/985431/max-parallel-http-connections-in-a-browser>
    fn default() -> Self {
        Self(6)
    }
}

impl MaxParallelDownloads {
    /// Use custom value.
    ///
    /// Many services have rate limits, and exceeding them may result in throttling, bans, or
    /// degraded service. You are **strongly encouraged** to check the Terms of Use of the
    /// particular provider you are using.
    pub fn value_manually_confirmed_with_provider_limits(value: usize) -> Self {
        Self(value)
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use native::*;

#[cfg(target_arch = "wasm32")]
pub(crate) use web::*;

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::{HttpOptions, bare_client};
    use http_cache_reqwest::{Cache, CacheMode, HttpCache, HttpCacheOptions, RedbManager};
    use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};

    /// File name of the redb database inside `HttpOptions::cache`.
    const CACHE_FILE: &str = "http-cache.redb";

    pub fn http_client(http_options: &HttpOptions) -> ClientWithMiddleware {
        let builder = ClientBuilder::new(bare_client(http_options));

        if let Some(manager) = http_options.cache.as_deref().and_then(open_cache) {
            builder.with(Cache(HttpCache {
                mode: CacheMode::Default,
                manager,
                options: HttpCacheOptions::default(),
            }))
        } else {
            builder
        }
        .build()
    }

    /// Opens (or creates) the cache database in `dir`.
    ///
    /// redb keeps one file under an exclusive lock, so a second client (or process) pointed at the
    /// same directory cannot open it. That client then runs without a cache instead of failing.
    fn open_cache(dir: &std::path::Path) -> Option<RedbManager> {
        let file = dir.join(CACHE_FILE);
        let opened = std::fs::create_dir_all(dir)
            .map_err(|e| e.to_string())
            .and_then(|()| RedbManager::new(&file).map_err(|e| e.to_string()));
        match opened {
            Ok(manager) => Some(manager),
            Err(e) => {
                log::warn!("HTTP cache disabled: cannot open {}: {e}", file.display());
                None
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    use super::{HttpOptions, bare_client};
    use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};

    pub fn http_client(http_options: &HttpOptions) -> ClientWithMiddleware {
        if http_options.cache.is_some() {
            log::warn!(
                "HTTP cache directory set, but ignored because, in WASM, caching is handled by the browser."
            );
        }
        ClientBuilder::new(bare_client(http_options)).build()
    }
}

fn bare_client(http_options: &HttpOptions) -> reqwest::Client {
    let mut builder = reqwest::Client::builder();

    if let Some(user_agent) = &http_options.user_agent {
        builder = builder.user_agent(user_agent);
    }

    #[cfg(not(target_arch = "wasm32"))]
    if let Some(tls) = &http_options.tls {
        builder = builder.tls_backend_preconfigured(tls.clone());
    }

    builder
        .build()
        .expect("could not initialize reqwest client")
}
