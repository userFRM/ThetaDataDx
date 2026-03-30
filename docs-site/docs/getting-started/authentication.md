---
title: Authentication
description: Configure ThetaData credentials for ThetaDataDx using a file or environment variables.
---

# Authentication

ThetaDataDx authenticates with ThetaData's servers using your account email and password. There are two ways to provide credentials.

## Credentials File

Create a `creds.txt` file with your ThetaData email on line 1 and password on line 2:

```text
your-email@example.com
your-password
```

::: warning
Do not commit `creds.txt` to version control. Add it to your `.gitignore`.
:::

Load the file in your application:

::: code-group
```rust [Rust]
let creds = Credentials::from_file("creds.txt")?;
```
```python [Python]
from thetadatadx import Credentials

creds = Credentials.from_file("creds.txt")
```
```go [Go]
creds, err := thetadatadx.CredentialsFromFile("creds.txt")
if err != nil {
    log.Fatal(err)
}
defer creds.Close()
```
```cpp [C++]
auto creds = tdx::Credentials::from_file("creds.txt");
```
:::

## Environment Variables

For containerized deployments or CI pipelines, pass credentials through environment variables:

::: code-group
```rust [Rust]
let creds = Credentials::new(
    std::env::var("THETA_EMAIL")?,
    std::env::var("THETA_PASS")?,
);
```
```python [Python]
import os
from thetadatadx import Credentials

creds = Credentials(os.environ["THETA_EMAIL"], os.environ["THETA_PASS"])
```
```go [Go]
creds, err := thetadatadx.CredentialsFromEnv("THETA_EMAIL", "THETA_PASS")
if err != nil {
    log.Fatal(err)
}
defer creds.Close()
```
```cpp [C++]
auto creds = tdx::Credentials(
    std::getenv("THETA_EMAIL"),
    std::getenv("THETA_PASS")
);
```
:::

::: tip
Environment variables are the recommended approach for production deployments and Docker containers. The file-based approach is convenient for local development.
:::

## Connecting

Once you have credentials, create a client connected to ThetaData's production servers:

::: code-group
```rust [Rust]
use thetadatadx::{ThetaDataDx, Credentials, DirectConfig};

let creds = Credentials::from_file("creds.txt")?;
let client = ThetaDataDx::connect(&creds, DirectConfig::production()).await?;
```
```python [Python]
from thetadatadx import Credentials, Config, ThetaDataDx

creds = Credentials.from_file("creds.txt")
client = ThetaDataDx(creds, Config.production())
```
```go [Go]
creds, _ := thetadatadx.CredentialsFromFile("creds.txt")
defer creds.Close()

config := thetadatadx.ProductionConfig()
defer config.Close()

client, err := thetadatadx.Connect(creds, config)
if err != nil {
    log.Fatal(err)
}
defer client.Close()
```
```cpp [C++]
auto creds = tdx::Credentials::from_file("creds.txt");
auto client = tdx::Client::connect(creds, tdx::Config::production());
```
:::

The client authenticates automatically on connection. If credentials are invalid, the connection call returns an error immediately.
