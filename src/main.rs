mod utils;
mod socks;

use log::LevelFilter;
use net2::TcpStreamExt;
use simple_logger::SimpleLogger;
use getopts::Options;
use tokio::{io::{self, AsyncWriteExt, AsyncReadExt}, task, net::{TcpListener, TcpStream}};
use utils::MAGIC_FLAG;

fn usage(program: &str, opts: &Options) {
    let program_path = std::path::PathBuf::from(program);
    let program_name = program_path.file_stem().unwrap().to_str().unwrap();
    let brief = format!("Usage: {} [-ltr] [IP_ADDRESS] [-s] [IP_ADDRESS]",
                        program_name);
    print!("{}", opts.usage(&brief));
}

#[derive(Debug, Clone)]
struct SlaveNode {
	address: String,
	weight: u32,
	current_weight: u32,
}

#[tokio::main]
async fn main() -> io::Result<()>  {
	SimpleLogger::new().with_utc_timestamps().with_utc_timestamps().with_colors(true).init().unwrap();
	::log::set_max_level(LevelFilter::Info);

    let args: Vec<String> = std::env::args().collect();
    let program = args[0].clone();

    let mut opts = Options::new();

    opts.optopt("l", "bind", "The address on which to listen socks5 server for incoming requests", "BIND_ADDR");
	opts.optopt("t", "transfer", "The address accept from slave socks5 server connection", "TRANSFER_ADDRESS");
	opts.optopt("r", "reverse", "reverse socks5 server connect to master", "TRANSFER_ADDRESS");
	opts.optopt("s", "server", "The address on which to listen local socks5 server", "TRANSFER_ADDRESS");

	let matches = opts.parse(&args[1..]).unwrap_or_else(|_| {
		usage(&program, &opts);
		std::process::exit(-1);
	});

	if matches.opt_count("t") > 0 {
		let master_addr : String = match match matches.opt_str("t"){
			Some(p) => p,
			None => {
				log::error!("not found listen port . eg : rsocx -t 0.0.0.0:8000 -s 0.0.0.0:1080");
				return Ok(());
			},
		}.parse(){
			Err(_) => {
				log::error!("not found listen port . eg : rsocx -t 0.0.0.0:8000 -s 0.0.0.0:1080");
				return Ok(());
			},
			Ok(p) => p
		};
		let socks_addr : String = match match matches.opt_str("s"){
			Some(p) => p,
			None => {
				log::error!("not found listen port . eg : rsocx -t 0.0.0.0:8000 -s 0.0.0.0:1080");
				return Ok(());
			},
		}.parse(){
			Err(_) => {
				log::error!("not found listen port . eg : rsocx -t 0.0.0.0:8000 -s 0.0.0.0:1080");
				return Ok(());
			},
			Ok(p) => p
		};

		//List of connected slave nodes
		let slaves = Arc:new(Mutex::new(Vec::new()));
		let slave_listener = TcpListener::bind(&master_addr).await?;
		let listener = TcpListener::bind(&master_addr).await?;

		log::info!("Master listening on {} for slaves", master_addr);
        log::info!("Master listening on {} for client connections", socks_addr);

		// Task to accept and manage slave nodes
		let slave_handler = {
            let slaves = slaves.clone();
            tokio::spawn(async move {
                loop {
                    let (slave_stream, slave_addr) = slave_listener.accept().await.unwrap();
                    let raw_stream = slave_stream.into_std().unwrap();
                    raw_stream.set_keepalive(Some(std::time::Duration::from_secs(10))).unwrap();
                    let slave_stream = TcpStream::from_std(raw_stream).unwrap();

                    log::info!("Slave connected from {}", slave_addr);

                    // Add the new slave connection to the list
                    slaves.lock().unwrap().push(slave_stream);
                }
            })
        };

		// Index to keep track of the next slave node for round-robin distribution
        let mut slave_index = 0;

		// Task to handle client connections and distribute them to slave nodes
        let client_handler = tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let raw_stream = stream.into_std().unwrap();
                raw_stream.set_keepalive(Some(std::time::Duration::from_secs(10))).unwrap();
                let stream = TcpStream::from_std(raw_stream).unwrap();

                // Get the current list of slaves
                let mut slaves_guard = slaves.lock().unwrap();

                // Check if there are any slave nodes connected
                if slaves_guard.is_empty() {
                    log::error!("No slave nodes connected to handle the client.");
                    continue;
                }

                // Select the next slave node in round-robin fashion
                let selected_slave = &mut slaves_guard[slave_index];
                slave_index = (slave_index + 1) % slaves_guard.len();  // Round-robin logic

                // Spawn a task to relay traffic between the client and the selected slave node
                tokio::spawn(relay_traffic(stream, selected_slave.clone()));
            }
        });

		// Wait for both tasks to complete
        let _ = tokio::join!(slave_handler, client_handler);
	} else {
		usage(&program, &opts);
	}
	Ok(())
}

// Function to relay traffic between client and slave node
async fn relay_traffic(mut client_stream: TcpStream, mut slave_stream: TcpStream) -> io::Result<()> {
    let (mut client_read, mut client_write) = client_stream.split();
    let (mut slave_read, mut slave_write) = slave_stream.split();

    let client_to_slave = async {
        io::copy(&mut client_read, &mut slave_write).await?;
        slave_write.shutdown().await
    };

    let slave_to_client = async {
        io::copy(&mut slave_read, &mut client_write).await?;
        client_write.shutdown().await
    };

    tokio::try_join!(client_to_slave, slave_to_client)?;

    Ok(())
}
