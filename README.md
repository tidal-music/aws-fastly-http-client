# Fastly Http Client for the AWS Rust SDK
The `aws-fastly-http-client` crate allows you to use the AWS Rust SDK on Fastly Compute @ Edge. The crate contains an
implementation of [HttpClient](https://docs.rs/aws-sdk-config/latest/aws_sdk_config/config/trait.HttpClient.html)
you can plug into the AWS Rust SDK. We've only used this with the [aws-sdk-dynamodb](https://crates.io/crates/aws-sdk-dynamodb)
but it should work with other services as well.

## Blocking
Sadly this HTTP client blocks. So you can't send two parallel Query requests to DynamoDB. The blocking version of the
HTTP client got the job done for us, but feel free to help out and make it async.

## Dependencies
We found that we needed to disable all default features of the AWS Rust SDK to make our project build for Fastly
Compute@Edge. For example, if you're adding the `aws-sdk-dynamodb` crate, you'll need to add this to your `Cargo.toml`:

```toml
aws-sdk-dynamodb = { version = "1.9.0", default-features = false }
```

## Usage
AWS's Rust SDK allows you to control a lot of stuff related to networking, but since Fastly handles that for us, most of
the networking things must be disabled. Here's an example of a `SdkConfig` that worked for us:
```rust
fn dynamodb_client() -> aws_sdk_dynamodb::Client {
    let sender = DefaultSender::from("my_backend_name");
    let http_client = FastlyHttpClient::from(sender);
    
    let config = SdkConfig::builder()
        .region(region())
        .credentials_provider(credentials_provider())
        .retry_config(RetryConfig::disabled())
        .timeout_config(TimeoutConfig::disabled())
        .stalled_stream_protection(StalledStreamProtectionConfig::disabled())
        .identity_cache(IdentityCache::no_cache())
        .http_client(http_client)
        .behavior_version(BehaviorVersion::v2023_11_09())
        .build();

    aws_sdk_dynamodb::Client::new(&config)
}
```

