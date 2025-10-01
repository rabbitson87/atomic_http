use atomic_http::{ConnectionPoolConfig, Options, SendableError, Server};

#[tokio::main]
async fn main() -> Result<(), SendableError> {
    println!("🔧 Options-based Configuration Examples");
    println!("=======================================\n");

    // Example 1: Server with default options (no connection pool)
    println!("1. Creating server with default options...");
    let server1 = Server::new("127.0.0.1:8080").await?;
    #[cfg(feature = "connection_pool")]
    {
        if server1.options.is_connection_pool_enabled() {
            println!("   Connection pool enabled (default)");
        } else {
            println!("   Connection pool disabled");
        }
    }

    // Example 2: Custom options with connection pool
    println!("\n2. Creating server with custom options...");
    let mut options = Options::new();

    #[cfg(feature = "connection_pool")]
    {
        let pool_config = ConnectionPoolConfig::new()
            .max_connections_per_host(64)
            .idle_timeout(120)
            .max_lifetime(900)
            .keep_alive(true);

        options.set_connection_option(pool_config);
        println!(
            "   Connection pool configured: {:?}",
            options.get_connection_option()
        );
    }

    let mut server2 = Server::with_options("127.0.0.1:8081", options).await?;
    #[cfg(feature = "connection_pool")]
    server2.print_connection_pool_stats().await;

    // Example 3: Options with high-performance preset
    println!("\n3. Creating server with high-performance options...");
    let mut hp_options = Options::new();

    #[cfg(feature = "connection_pool")]
    {
        hp_options.set_connection_option(ConnectionPoolConfig::high_performance());
    }

    let mut server3 = Server::with_options("127.0.0.1:8082", hp_options).await?;
    #[cfg(feature = "connection_pool")]
    server3.print_connection_pool_stats().await;

    // Example 4: Runtime option changes
    println!("\n4. Runtime option changes...");
    let mut runtime_options = Options::new();

    println!("   Initially connection pool enabled:");
    #[cfg(feature = "connection_pool")]
    println!(
        "   Pool enabled: {}",
        runtime_options.is_connection_pool_enabled()
    );

    #[cfg(feature = "connection_pool")]
    {
        println!("   Disabling connection pool...");
        runtime_options.disable_connection_pool();
        println!(
            "   Pool enabled: {}",
            runtime_options.is_connection_pool_enabled()
        );

        println!("   Re-enabling connection pool...");
        runtime_options.enable_connection_pool();
        println!(
            "   Pool enabled: {}",
            runtime_options.is_connection_pool_enabled()
        );
    }

    // Example 5: Environment-based configuration
    println!("\n5. Environment-based configuration...");

    // Set some environment variables (in real usage, these would be set externally)
    std::env::set_var("ENABLE_KEEP_ALIVE", "true");
    std::env::set_var("KEEP_ALIVE_TIMEOUT", "90");
    std::env::set_var("MAX_IDLE_CONNECTIONS_PER_HOST", "50");

    let env_options = Options::new(); // This will read from environment
    #[cfg(feature = "connection_pool")]
    {
        let config = env_options.get_connection_option();
        println!(
            "   Environment config loaded: max_connections_per_host={}, idle_timeout={}s",
            config.max_connections_per_host,
            config.max_idle_time.as_secs()
        );
    }

    let mut env_server = Server::with_options("127.0.0.1:8083", env_options).await?;
    #[cfg(feature = "connection_pool")]
    env_server.print_connection_pool_stats().await;

    // Cleanup
    println!("\n🧹 Cleaning up servers...");
    #[cfg(feature = "connection_pool")]
    {
        server2.disable_connection_pool().await;
        server3.disable_connection_pool().await;
        env_server.disable_connection_pool().await;
    }

    println!("\n✅ All options-based configuration examples completed!");
    println!("\n📋 Key Benefits:");
    println!("   ✓ Centralized configuration in Options struct");
    println!("   ✓ Connection pooling enabled by default when feature is active");
    println!("   ✓ Environment variable support");
    println!("   ✓ Runtime configuration changes");
    println!("   ✓ Type-safe configuration with builder pattern");
    println!("   ✓ Automatic connection pool initialization");

    Ok(())
}
