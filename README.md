# Fastly Http Client for the AWS Rust SDK
The `aws-fastly-http-client` crate allows you to use the AWS Rust SDK on Fastly Compute @ Edge. The crate contains an
implementation of [HttpClient](https://docs.rs/aws-sdk-config/latest/aws_sdk_config/config/trait.HttpClient.html)
you can plug into the AWS Rust SDK. We've only used this with the [aws-sdk-dynamodb](https://crates.io/crates/aws-sdk-dynamodb)
but it should work with other services as well.

## Dependencies
We found that we needed to disable all default features of the AWS Rust SDK to make our project build for Fastly
Compute@Edge. For example, if you're adding the `aws-sdk-dynamodb` crate, you'll need to add this to your `Cargo.toml`:

```toml
aws-sdk-dynamodb = { version = "1.9.0", default-features = false }
```

Additionally, we rely on Tokio for some of the async stuff, so you might want to add this to your `Cargo.toml` and use
the `tokio::main` macro.
```toml
tokio = { version = "1.35.1", features = ["macros", "rt"] }
```

## Usage
AWS's Rust SDK allows you to control a lot of stuff related to networking, but since Fastly handles that for us, most of
the networking things can be disabled. Here's an example with a `SdkConfig` that worked for us:

```rust
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let http_client = FastlyHttpClient::from("my_backend_name");
    let config = aws_sdk_dynamodb::Config::builder()
        .region(Some(Region::from_static("us-east-1")))
        .credentials_provider(credentials_provider())
        .retry_config(RetryConfig::disabled())
        .timeout_config(TimeoutConfig::disabled())
        .stalled_stream_protection(StalledStreamProtectionConfig::disabled())
        .identity_cache(IdentityCache::no_cache())
        .http_client(http_client)
        .behavior_version(BehaviorVersion::v2023_11_09())
        .build();

    let client = aws_sdk_dynamodb::Client::from_conf(config);

    let request = Request::from_client();
    let result = client
        .get_item()
        .table_name("paths")
        .key("path", AttributeValue::S(request.get_path().to_string()))
        .send()
        .await
        .unwrap();

    let response = match result.item {
        Some(_) => Response::from_status(StatusCode::OK),
        None => Response::from_status(StatusCode::NOT_FOUND),
    };

    response.send_to_client()
}
```