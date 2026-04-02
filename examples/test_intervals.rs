use thetadatadx::{Credentials, DirectConfig, ThetaDataDx};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let creds = Credentials::from_file("/home/theta-gamma/thetadx/creds.txt")?;
    let tdx = ThetaDataDx::connect(&creds, DirectConfig::production()).await?;
    println!("Connected: {}\n", tdx.session_uuid());

    let tests = vec![
        // (description, endpoint, args)
        ("stock_history_ohlc AAPL '60000'", "stock", "60000"),
        ("stock_history_ohlc AAPL '1s'", "stock", "1s"),
        ("stock_history_ohlc AAPL '00:01:00.000'", "stock", "00:01:00.000"),
        ("index_history_ohlc SPX '60000'", "index", "60000"),
        ("index_history_ohlc SPX '1s'", "index", "1s"),
        ("index_history_ohlc SPX '00:01:00.000'", "index", "00:01:00.000"),
        ("index_history_ohlc SPX '1m'", "index", "1m"),
        ("index_history_ohlc SPX '60'", "index", "60"),
        ("index_history_ohlc SPX '1'", "index", "1"),
    ];

    for (desc, endpoint, interval) in tests {
        print!("{}: ", desc);
        let result = match endpoint {
            "stock" => tdx.stock_history_ohlc("AAPL", "20260325", interval).await,
            "index" => tdx.index_history_ohlc("SPX", "20260325", "20260325", interval).await,
            _ => unreachable!(),
        };
        match result {
            Ok(ticks) => println!("{} ticks", ticks.len()),
            Err(e) => println!("ERROR: {}", e),
        }
    }

    Ok(())
}
