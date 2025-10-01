use dashmap::DashMap;
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio::sync::Mutex;

use crate::dev_print;
use crate::SendableError;

/// Connection statistics
#[derive(Debug, Clone)]
pub struct ConnectionStats {
    pub total_connections: u64,
    pub active_connections: u64,
    pub pooled_connections: u64,
    pub reused_connections: u64,
    pub connection_hits: u64,
    pub connection_misses: u64,
}

impl std::fmt::Display for ConnectionStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Connections: {} total, {} active, {} pooled, {} reused (hit rate: {:.1}%)",
            self.total_connections,
            self.active_connections,
            self.pooled_connections,
            self.reused_connections,
            if self.connection_hits + self.connection_misses > 0 {
                self.connection_hits as f64 / (self.connection_hits + self.connection_misses) as f64
                    * 100.0
            } else {
                0.0
            }
        )
    }
}

/// Connection metadata
#[derive(Debug)]
struct ConnectionMetadata {
    created_at: Instant,
    last_used: Instant,
    total_requests: u64,
}

impl ConnectionMetadata {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            created_at: now,
            last_used: now,
            total_requests: 0,
        }
    }

    fn update_last_used(&mut self) {
        self.last_used = Instant::now();
        self.total_requests += 1;
    }

    fn is_expired(&self, max_idle_time: Duration, max_lifetime: Duration) -> bool {
        let now = Instant::now();
        now.duration_since(self.last_used) > max_idle_time
            || now.duration_since(self.created_at) > max_lifetime
    }
}

/// Pooled connection wrapper
#[derive(Debug)]
struct PooledConnection {
    stream: TcpStream,
    metadata: ConnectionMetadata,
}

impl PooledConnection {
    fn new(stream: TcpStream) -> Self {
        Self {
            stream,
            metadata: ConnectionMetadata::new(),
        }
    }

    fn update_usage(&mut self) {
        self.metadata.update_last_used();
    }

    fn is_expired(&self, max_idle_time: Duration, max_lifetime: Duration) -> bool {
        self.metadata.is_expired(max_idle_time, max_lifetime)
    }
}

/// Connection pool configuration
#[derive(Debug, Clone)]
pub struct ConnectionPoolConfig {
    pub max_connections_per_host: usize,
    pub max_idle_time: Duration,
    pub max_lifetime: Duration,
    pub cleanup_interval: Duration,
    pub enable_keep_alive: bool,
}

impl Default for ConnectionPoolConfig {
    fn default() -> Self {
        Self {
            max_connections_per_host: 32,              // nginx upstream default
            max_idle_time: Duration::from_secs(75),    // nginx keepalive_timeout default
            max_lifetime: Duration::from_secs(600),    // 10 minutes, nginx-like
            cleanup_interval: Duration::from_secs(30), // more frequent cleanup
            enable_keep_alive: true,
        }
    }
}

impl ConnectionPoolConfig {
    /// Create new config with nginx-like defaults
    pub fn new() -> Self {
        Self::default()
    }

    /// Set maximum connections per host (nginx upstream keepalive)
    pub fn max_connections_per_host(mut self, max_connections: usize) -> Self {
        self.max_connections_per_host = max_connections;
        self
    }

    /// Set idle timeout (nginx keepalive_timeout)
    pub fn idle_timeout(mut self, timeout_secs: u64) -> Self {
        self.max_idle_time = Duration::from_secs(timeout_secs);
        self
    }

    /// Set connection lifetime (nginx keepalive_time)
    pub fn max_lifetime(mut self, lifetime_secs: u64) -> Self {
        self.max_lifetime = Duration::from_secs(lifetime_secs);
        self
    }

    /// Set cleanup interval
    pub fn cleanup_interval(mut self, interval_secs: u64) -> Self {
        self.cleanup_interval = Duration::from_secs(interval_secs);
        self
    }

    /// Enable or disable keep-alive
    pub fn keep_alive(mut self, enable: bool) -> Self {
        self.enable_keep_alive = enable;
        self
    }

    /// Preset: High performance (more connections, longer timeouts)
    pub fn high_performance() -> Self {
        Self {
            max_connections_per_host: 128,
            max_idle_time: Duration::from_secs(300), // 5 minutes
            max_lifetime: Duration::from_secs(1800), // 30 minutes
            cleanup_interval: Duration::from_secs(60),
            enable_keep_alive: true,
        }
    }

    /// Preset: Conservative (fewer connections, shorter timeouts)
    pub fn conservative() -> Self {
        Self {
            max_connections_per_host: 16,
            max_idle_time: Duration::from_secs(30),
            max_lifetime: Duration::from_secs(300), // 5 minutes
            cleanup_interval: Duration::from_secs(15),
            enable_keep_alive: true,
        }
    }

    /// Create with default configuration (same as default but more explicit)
    pub fn default_config() -> Self {
        Self::default()
    }

    /// Disable connection pooling
    pub fn disabled() -> Self {
        Self {
            max_connections_per_host: 0,
            max_idle_time: Duration::from_secs(0),
            max_lifetime: Duration::from_secs(0),
            cleanup_interval: Duration::from_secs(30),
            enable_keep_alive: false,
        }
    }
}

/// High-performance connection pool
pub struct ConnectionPool {
    // Connection pools per host
    pools: Arc<DashMap<SocketAddr, Arc<Mutex<VecDeque<PooledConnection>>>>>,

    // Configuration
    config: ConnectionPoolConfig,

    total_connections: AtomicU64,
    active_connections: AtomicU64,
    reused_connections: AtomicU64,
    connection_hits: AtomicU64,
    connection_misses: AtomicU64,

    // Cleanup task handle
    cleanup_handle: Option<tokio::task::JoinHandle<()>>,
}

impl ConnectionPool {
    pub fn new(config: ConnectionPoolConfig) -> Self {
        let pool = Self {
            pools: Arc::new(DashMap::new()),
            config: config.clone(),
            total_connections: AtomicU64::new(0),
            active_connections: AtomicU64::new(0),
            reused_connections: AtomicU64::new(0),
            connection_hits: AtomicU64::new(0),
            connection_misses: AtomicU64::new(0),
            cleanup_handle: None,
        };

        pool
    }

    /// Start the cleanup task
    pub fn start_cleanup_task(&mut self) {
        let pools = self.pools.clone();
        let config = self.config.clone();

        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(config.cleanup_interval);

            loop {
                interval.tick().await;
                Self::cleanup_expired_connections(&pools, &config).await;
            }
        });

        self.cleanup_handle = Some(handle);
    }

    /// Get a connection from the pool or create a new one
    pub async fn get_connection(&self, addr: SocketAddr) -> Result<TcpStream, SendableError> {
        // Try to get from pool first
        if let Some(pool) = self.pools.get(&addr) {
            let mut pool_guard = pool.lock().await;

            while let Some(mut conn) = pool_guard.pop_front() {
                if !conn.is_expired(self.config.max_idle_time, self.config.max_lifetime) {
                    conn.update_usage();

                    self.connection_hits.fetch_add(1, Ordering::Relaxed);
                    self.reused_connections.fetch_add(1, Ordering::Relaxed);

                    dev_print!("Connection pool hit for {}: reusing connection", addr);
                    return Ok(conn.stream);
                } else {
                    dev_print!("Connection pool: expired connection removed for {}", addr);
                }
            }
        }

        // Pool miss - create new connection
        self.connection_misses.fetch_add(1, Ordering::Relaxed);
        self.total_connections.fetch_add(1, Ordering::Relaxed);
        self.active_connections.fetch_add(1, Ordering::Relaxed);

        dev_print!("Connection pool miss for {}: creating new connection", addr);

        let stream = TcpStream::connect(addr).await?;

        // Set TCP_NODELAY for better performance
        if let Err(e) = stream.set_nodelay(true) {
            dev_print!("Failed to set TCP_NODELAY: {}", e);
        }

        Ok(stream)
    }

    /// Return a connection to the pool
    pub async fn return_connection(&self, addr: SocketAddr, stream: TcpStream, keep_alive: bool) {
        if !keep_alive || !self.config.enable_keep_alive {
            self.active_connections.fetch_sub(1, Ordering::Relaxed);
            dev_print!(
                "Connection not returned to pool (keep_alive={})",
                keep_alive
            );
            return;
        }

        // Get or create pool for this address
        let pool = self
            .pools
            .entry(addr)
            .or_insert_with(|| Arc::new(Mutex::new(VecDeque::new())))
            .clone();

        let mut pool_guard = pool.lock().await;

        // Check if pool is full
        if pool_guard.len() >= self.config.max_connections_per_host {
            self.active_connections.fetch_sub(1, Ordering::Relaxed);
            dev_print!("Connection pool full for {}: dropping connection", addr);
            return;
        }

        // Add to pool
        let pooled_conn = PooledConnection::new(stream);
        pool_guard.push_back(pooled_conn);

        self.active_connections.fetch_sub(1, Ordering::Relaxed);

        dev_print!(
            "Connection returned to pool for {}: {} connections pooled",
            addr,
            pool_guard.len()
        );
    }

    /// Cleanup expired connections
    async fn cleanup_expired_connections(
        pools: &Arc<DashMap<SocketAddr, Arc<Mutex<VecDeque<PooledConnection>>>>>,
        config: &ConnectionPoolConfig,
    ) {
        let mut total_cleaned = 0;

        for entry in pools.iter() {
            let addr = *entry.key();
            let pool = entry.value().clone();
            let mut pool_guard = pool.lock().await;

            let initial_size = pool_guard.len();
            pool_guard.retain(|conn| !conn.is_expired(config.max_idle_time, config.max_lifetime));

            let cleaned = initial_size - pool_guard.len();
            if cleaned > 0 {
                total_cleaned += cleaned;
                dev_print!("Cleaned {} expired connections for {}", cleaned, addr);
            }

            // Remove empty pools
            if pool_guard.is_empty() {
                drop(pool_guard);
                pools.remove(&addr);
            }
        }

        if total_cleaned > 0 {
            dev_print!(
                "Connection pool cleanup: removed {} expired connections",
                total_cleaned
            );
        }
    }

    /// Get current connection statistics
    pub fn stats(&self) -> ConnectionStats {
        let total_pooled = self
            .pools
            .iter()
            .map(|entry| {
                // We can't block here, so we'll use try_lock
                if let Ok(guard) = entry.value().try_lock() {
                    guard.len() as u64
                } else {
                    0
                }
            })
            .sum();

        ConnectionStats {
            total_connections: self.total_connections.load(Ordering::Relaxed),
            active_connections: self.active_connections.load(Ordering::Relaxed),
            pooled_connections: total_pooled,
            reused_connections: self.reused_connections.load(Ordering::Relaxed),
            connection_hits: self.connection_hits.load(Ordering::Relaxed),
            connection_misses: self.connection_misses.load(Ordering::Relaxed),
        }
    }

    /// Clear all pooled connections
    pub async fn clear(&self) {
        let mut total_cleared = 0;

        for entry in self.pools.iter() {
            let pool = entry.value().clone();
            let mut pool_guard = pool.lock().await;
            total_cleared += pool_guard.len();
            pool_guard.clear();
        }

        self.pools.clear();

        dev_print!(
            "Connection pool cleared: {} connections removed",
            total_cleared
        );
    }

    /// Shutdown the connection pool
    pub async fn shutdown(&mut self) {
        if let Some(handle) = self.cleanup_handle.take() {
            handle.abort();
        }

        self.clear().await;
        dev_print!("Connection pool shutdown completed");
    }
}

impl Drop for ConnectionPool {
    fn drop(&mut self) {
        if let Some(handle) = self.cleanup_handle.take() {
            handle.abort();
        }
    }
}

// Global connection pool instance
use std::sync::OnceLock;
static GLOBAL_CONNECTION_POOL: OnceLock<Arc<Mutex<ConnectionPool>>> = OnceLock::new();

impl ConnectionPool {
    /// Initialize global connection pool
    pub fn init_global(config: Option<ConnectionPoolConfig>) {
        let config = config.unwrap_or_default();
        let mut pool = ConnectionPool::new(config);
        pool.start_cleanup_task();

        let _ = GLOBAL_CONNECTION_POOL.set(Arc::new(Mutex::new(pool)));
        dev_print!("Global connection pool initialized");
    }

    /// Get global connection pool
    pub async fn global() -> Arc<Mutex<ConnectionPool>> {
        GLOBAL_CONNECTION_POOL
            .get_or_init(|| {
                let mut pool = ConnectionPool::new(ConnectionPoolConfig::default());
                pool.start_cleanup_task();
                Arc::new(Mutex::new(pool))
            })
            .clone()
    }
}
