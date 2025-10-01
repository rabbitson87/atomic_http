use atomic_http::{ConnectionPoolConfig, Options, SendableError, Server};

/// Production-ready server setup with proper configuration
#[tokio::main]
async fn main() -> Result<(), SendableError> {
    println!("🏭 Production Server Setup Example");
    println!("==================================\n");

    // Step 1: Create production-ready options
    println!("1. Setting up production options...");
    let options = setup_production_options();

    // Step 2: Create server with options
    println!("2. Creating server with production configuration...");
    let mut server = Server::with_options("127.0.0.1:8080", options).await?;

    println!("   Server listening on http://127.0.0.1:8080");

    #[cfg(feature = "connection_pool")]
    {
        server.print_connection_pool_stats().await;
    }

    // Step 3: Start handling requests (simulation)
    println!("\n3. Starting request handling simulation...");

    // Simulate some configuration scenarios
    demonstrate_configuration_scenarios().await?;

    println!("\n✅ Production server setup completed!");

    #[cfg(feature = "connection_pool")]
    {
        println!("\n🧹 Shutting down connection pool...");
        server.disable_connection_pool().await;
    }

    Ok(())
}

fn setup_production_options() -> Options {
    let mut options = Options::new();

    // Basic server configuration
    options.no_delay = true;
    options.read_timeout_miliseconds = 5000; // 5 second timeout
    options.read_buffer_size = 8192; // 8KB buffer
    options.zero_copy_threshold = 512 * 1024; // 512KB threshold
    options.enable_file_cache = true;

    // Set root path for static files
    if let Ok(current_dir) = std::env::current_dir() {
        options.root_path = current_dir.join("static");
    }

    // Connection pool configuration for production
    #[cfg(feature = "connection_pool")]
    {
        let pool_config = ConnectionPoolConfig::new()
            .max_connections_per_host(100) // nginx-like
            .idle_timeout(75) // 75 seconds
            .max_lifetime(600) // 10 minutes
            .cleanup_interval(30) // cleanup every 30 seconds
            .keep_alive(true);

        options.set_connection_option(pool_config);
        println!("   ✓ Connection pooling enabled");
    }

    println!("   ✓ Production options configured");
    options
}

async fn demonstrate_configuration_scenarios() -> Result<(), SendableError> {
    println!("\n📊 Configuration Scenarios:");

    // Scenario 1: High-traffic server
    println!("\n   Scenario 1: High-traffic server setup");
    let mut high_traffic_options = Options::new();

    #[cfg(feature = "connection_pool")]
    {
        high_traffic_options.set_connection_option(ConnectionPoolConfig::high_performance());

        let _server = Server::with_options("127.0.0.1:8090", high_traffic_options).await?;
        println!("      ✓ High-performance server created");
        // Server would handle high traffic with 128 connections per host
    }

    // Scenario 2: Resource-constrained server
    println!("\n   Scenario 2: Resource-constrained server setup");
    let mut conservative_options = Options::new();

    #[cfg(feature = "connection_pool")]
    {
        conservative_options.set_connection_option(ConnectionPoolConfig::conservative());

        let _server = Server::with_options("127.0.0.1:8091", conservative_options).await?;
        println!("      ✓ Conservative server created");
        // Server would use minimal resources with 16 connections per host
    }

    // Scenario 3: Development server (no pooling)
    println!("\n   Scenario 3: Development server setup");
    let dev_options = Options::new(); // No connection pool by default

    let _server = Server::with_options("127.0.0.1:8092", dev_options).await?;
    println!("      ✓ Development server created (no connection pooling)");

    // Scenario 4: Custom configuration
    println!("\n   Scenario 4: Custom business logic server");
    let mut custom_options = Options::new();

    #[cfg(feature = "connection_pool")]
    {
        let custom_config = ConnectionPoolConfig::new()
            .max_connections_per_host(50) // Medium scale
            .idle_timeout(120) // 2 minutes idle
            .max_lifetime(1800) // 30 minutes max life
            .cleanup_interval(60) // cleanup every minute
            .keep_alive(true);

        custom_options.set_connection_option(custom_config);

        let _server = Server::with_options("127.0.0.1:8093", custom_options).await?;
        println!("      ✓ Custom configuration server created");
    }

    println!("\n   All scenarios demonstrated successfully!");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_production_options() {
        let options = setup_production_options();

        // Test basic options
        assert_eq!(options.no_delay, true);
        assert_eq!(options.read_timeout_miliseconds, 5000);
        assert_eq!(options.read_buffer_size, 8192);
        assert_eq!(options.zero_copy_threshold, 512 * 1024);

        #[cfg(feature = "connection_pool")]
        {
            assert!(options.is_connection_pool_enabled());

            let config = options.get_connection_option();
            assert_eq!(config.max_connections_per_host, 100);
            assert_eq!(config.max_idle_time.as_secs(), 75);
            assert_eq!(config.enable_keep_alive, true);
        }
    }

    #[tokio::test]
    async fn test_server_creation_with_options() -> Result<(), SendableError> {
        let options = setup_production_options();
        let server = Server::with_options("127.0.0.1:0", options).await?; // Use port 0 for testing

        #[cfg(feature = "connection_pool")]
        {
            // Should have connection pool if feature is enabled
            assert!(server.connection_pool.is_some());
        }

        Ok(())
    }
}
