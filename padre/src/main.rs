use std::io;
use std::net::SocketAddr;
use std::process::exit;
use std::sync::{Arc, Mutex};
use std::thread;

#[macro_use]
extern crate lazy_static;
extern crate regex;
extern crate clap;
extern crate signal_hook;
extern crate tokio;
#[macro_use]
extern crate futures;
extern crate bytes;

use tokio::runtime::current_thread::Runtime;
use tokio::net::{TcpListener, TcpStream};
use tokio::prelude::*;
use futures::future::{self, Either};
use clap::{Arg, App, ArgMatches};
use signal_hook::iterator::Signals;

mod request;
mod debugger;
mod notifier;

fn get_config<'a>() -> ArgMatches<'a> {
    let app = App::new("VIM Padre")
        .version("0.1.0")
        .author("Steven Trotter <stevetrot@gmail.com>")
        .about("A tool for building, debugging and reverse engineering in VIM")
        .long_about("Interfaces with 'lldb' or a similar debugger to debug programs and communicate with the vim-padre VIM plugin in order to effectively use VIM as a debugging interface.")
        .arg(Arg::with_name("port")
                 .short("p")
                 .long("port")
                 .takes_value(true)
                 .help("specify port to run on"))
        .arg(Arg::with_name("host")
                 .short("h")
                 .long("host")
                 .takes_value(true)
                 .help("specify host to run on"))
        .arg(Arg::with_name("debugger")
                 .short("d")
                 .long("debugger")
                 .takes_value(true)
                 .help("specify debugger to use"))
        .arg(Arg::with_name("type")
                 .short("t")
                 .long("type")
                 .takes_value(true)
                 .help("specify debugger type from [lldb, node, java, python]"))
        .arg(Arg::with_name("debug_cmd")
                 .multiple(true)
                 .takes_value(true))
        .get_matches();
    app
}

fn get_connection(args: &ArgMatches) -> SocketAddr {
    let port = match args.value_of("port") {
        None => 12345,
        Some(s) => {
            match s.parse::<i32>() {
                Ok(n) => n,
                Err(_) => {
                    panic!("Can't understand port");
                }
            }
        }
    };

    let host = match args.value_of("host") {
        None => "0.0.0.0",
        Some(s) => s
    };

    return format!("{}:{}", host, port).parse::<SocketAddr>().unwrap();
}

fn install_signals(signals: Signals, debugger: Arc<Mutex<debugger::PadreServer>>) {
    thread::spawn(move || {
        for _ in signals.forever() {
            match debugger.lock() {
                Ok(s) => {
                    match s.debugger.lock() {
                        Ok(t) => t.stop(),
                        Err(err) => println!("Debugger not found: {}", err),
                    };
                },
                Err(err) => println!("Debug server not found: {}", err),
            };
            println!("Terminated!");
            exit(0);
        }
    });
}

fn main() -> io::Result<()> {

    let args = get_config();

    let connection_string = get_connection(&args);
    let listener = TcpListener::bind(&connection_string)
                               .expect(&format!("Can't open TCP listener on {}", connection_string));

    println!("Listening on {}", connection_string);

    let notifier_rc = Arc::new(Mutex::new(notifier::Notifier::new()));

    let debug_cmd: Vec<String> = args.values_of("debug_cmd")
                                     .expect("Can't find program to debug, please rerun with correct parameters")
                                     .map(|x| x.to_string())
                                     .collect::<Vec<String>>();

    let debugger_rc = Arc::new(
        Mutex::new(
            debugger::get_debugger(args.value_of("debugger"),
                                   args.value_of("type"),
                                   Arc::clone(&notifier_rc))
        )
    );

//    let thread_debugger = Arc::clone(&debugger_rc);

//    let debugger_arg = match args.value_of("debugger") {
//        Some(s) => s,
//        None => "lldb",
//    }.clone().to_string();

//    let signals = Signals::new(&[signal_hook::SIGINT, signal_hook::SIGTERM])?;
//    install_signals(signals, Arc::clone(&debugger_rc));

//    thread::spawn(move || {
//        thread_debugger.lock().unwrap().start(debugger_arg, &debug_cmd);
//    });

    let mut runtime = Runtime::new().unwrap();

    runtime.spawn(listener.incoming()
        .map_err(|e| eprintln!("failed to accept socket; error = {:?}", e))
        .for_each(move |socket| {
            let thread_debugger = Arc::clone(&debugger_rc);
            let thread_notifier = Arc::clone(&notifier_rc);

            let padre_connection = request::PadreConnection::new(socket, thread_notifier, thread_debugger);

            tokio::spawn(
                padre_connection
                    .into_future()
                    .map_err(|(e, _)| e)
                    .and_then(|_| {
                        // Dropped connection
                        future::ok(())
                    })
                    .map_err(|e| {
                        println!("connection error = {:?}", e);
                    })
            );

            Ok(())
        })
    );

    runtime.run().unwrap();

    Ok(())
}
