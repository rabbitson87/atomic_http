.PHONY: check build test examples bench clean all

ALL_FEATURES = arena,simd,vectored_io,connection_pool,response_file,debug,env

# === Core ===

check:
	cargo check --all-features

build:
	cargo build --all-features

test:
	cargo test --all-features

bench:
	cargo bench --all-features

clean:
	cargo clean

# === Examples (build only) ===

examples: example-comparative_benchmark \
	example-connection_pool_benchmark \
	example-connection_pool_config_test \
	example-debug_test \
	example-integrated_multipart_test \
	example-integrated_test \
	example-options_based_configuration \
	example-production_server_setup \
	example-simd_benchmark \
	example-simd_comparison \
	example-simple_benchmark_test \
	example-simple_connection_test \
	example-simple_performance_test \
	example-simple_server_test \
	example-simple_vectored_test \
	example-vectored_io_benchmark \
	example-zero_copy_test

example-comparative_benchmark:
	cargo build --example comparative_benchmark --features arena

example-connection_pool_benchmark:
	cargo build --example connection_pool_benchmark --features arena,connection_pool

example-connection_pool_config_test:
	cargo build --example connection_pool_config_test --features arena,connection_pool

example-debug_test:
	cargo build --example debug_test --features arena,debug,env

example-integrated_multipart_test:
	cargo build --example integrated_multipart_test --features arena,env

example-integrated_test:
	cargo build --example integrated_test --features arena,env,response_file

example-options_based_configuration:
	cargo build --example options_based_configuration --features connection_pool,env

example-production_server_setup:
	cargo build --example production_server_setup --features connection_pool,env

example-simd_benchmark:
	cargo build --example simd_benchmark --features arena,simd

example-simd_comparison:
	cargo build --example simd_comparison --features arena,simd

example-simple_benchmark_test:
	cargo build --example simple_benchmark_test --features arena,env

example-simple_connection_test:
	cargo build --example simple_connection_test --features arena,connection_pool,simd,vectored_io

example-simple_performance_test:
	cargo build --example simple_performance_test --features arena

example-simple_server_test:
	cargo build --example simple_server_test --features arena,env

example-simple_vectored_test:
	cargo build --example simple_vectored_test --features arena,simd,vectored_io

example-vectored_io_benchmark:
	cargo build --example vectored_io_benchmark --features arena,vectored_io

example-zero_copy_test:
	cargo build --example zero_copy_test --features arena,response_file

# === All ===

all: check test examples
