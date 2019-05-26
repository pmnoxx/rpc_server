extern crate mio;
extern crate redis;

use std::collections::HashMap;
use std::io;
use std::io::Read;
use std::io::Write;
use std::thread;
use std::time::{Duration, Instant};

use mio::{Events, Poll, PollOpt, Ready, Registration, Token};
use mio::event::Evented;
use mio::tcp::{TcpListener, TcpStream};
use redis::Commands;
use std::collections::hash_map::RandomState;

const MAX_EVENTS: usize = 16;
const TIMER_INTERVAL_SECONDS: u64 = 3;

/// An Evented implementation which implements a simple timer by indicating readiness at periodic
/// intervals.
struct PeriodicTimer {
    registration: Registration,
    set_readiness: mio::SetReadiness,
}

impl PeriodicTimer {
    /// Create a PeriodicTimer and begin signalling readiness at the specified interval.
    fn new(interval: u64) -> PeriodicTimer {
        let (registration, set_readiness) = Registration::new2();
        let set_readiness_clone = set_readiness.clone();
        // Note: This example code omits important things like arranging termination of the thread
        // when the PeriodicTimer is dropped.
        thread::spawn(move || loop {
            let now = Instant::now();
            let when = now + Duration::from_secs(interval);
            if now < when {
                thread::sleep(when - now);
            }
            set_readiness_clone
                .set_readiness(Ready::readable())
                .unwrap();
        });
        PeriodicTimer {
            registration,
            set_readiness,
        }
    }

    /// Clear the read readiness of this timer.
    fn reset(&self) {
        self.set_readiness.set_readiness(Ready::empty()).unwrap();
    }
}

/// Proxy Evented functions to the Registration.
impl Evented for PeriodicTimer {
    fn register(
        &self,
        poll: &Poll,
        token: Token,
        interest: Ready,
        opts: PollOpt,
    ) -> io::Result<()> {
        self.registration.register(poll, token, interest, opts)
    }
    fn reregister(
        &self,
        poll: &Poll,
        token: Token,
        interest: Ready,
        opts: PollOpt,
    ) -> io::Result<()> {
        self.registration.reregister(poll, token, interest, opts)
    }
    fn deregister(&self, poll: &Poll) -> io::Result<()> {
        <Registration as Evented>::deregister(&self.registration, poll)
    }
}

struct RpcServer {
    port: i32,
    connections: HashMap<Token, TcpStream, RandomState>,
    connection_id: usize,
    poll: Poll,
    num_rpc_count: i32
}

impl RpcServer {
    pub fn new(port: i32) -> RpcServer {
        RpcServer {
            port,
            connections: HashMap::new(),
            connection_id: 2,
            poll: Poll::new().unwrap(),
            num_rpc_count: 0
        }
    }

    fn print_stats(&mut self) {
        println!("Num rpc in last 3 second: {}", self.num_rpc_count);

        self.num_rpc_count = 0;
    }

    fn io_loop(&mut self) {

        // initialize pool
        let mut events = Events::with_capacity(MAX_EVENTS);

        // start listener
        let addr = format!("{}:{}", "127.0.0.1", self.port.to_string()).parse().unwrap();

        let server = TcpListener::bind(&addr).unwrap();
        self.poll.register(&server, Token(0), Ready::readable(), PollOpt::level())
            .unwrap();
        println!("Listening on port {}", self.port);

        // add timer example
        let timer = PeriodicTimer::new(TIMER_INTERVAL_SECONDS);
        self.poll.register(&timer, Token(1), Ready::readable(), PollOpt::level())
            .unwrap();

        fn fetch_an_integer() -> redis::RedisResult<isize> {
            // connect to redis
            let client = redis::Client::open("redis://127.0.0.1/")?;
            let con = client.get_connection()?;
            // throw away the result, just make sure it does not fail
            for _x in 1..1000000 {
                let _: () = con.set("my_key", _x)?;
            }
            // read back the key and return it.  Because the return value
            // from the function is a result for integer this will automatically
            // convert into one.
            con.get("my_key")
        }

        // Main loop
        loop {
            // Poll
            // println!("before poll()");
            self.poll.poll(&mut events, None).unwrap();
            // println!("after poll()");

            // Process events
            for event in &events {
                assert!(event.readiness().is_readable());
                if self.connections.contains_key(&event.token()) {
                    println!("hmm");
                    println!("fetch {}", fetch_an_integer().unwrap());

                    let mut stream = self.connections.get(&event.token()).unwrap();
                    let mut line = [0; 512];
                    stream.read(&mut line).unwrap();
                    stream.write(b"OK\n").unwrap();
                    self.num_rpc_count += 1;
                } else {
                    match event.token() {
                        Token(0) => {
                            // new connection
                            let result = server.accept();
                            println!("got tcp connection");
                            let (mut something, _s2) = result.unwrap();
                            something.write(b"hello\n").unwrap();

                            self.poll.register(&something, Token(2), Ready::readable(), PollOpt::level()).unwrap();

                            self.connections.insert(Token(self.connection_id), something);
                            self.connection_id += 1;
                        }
                        Token(1) => {
                            println!("{}-second timer", TIMER_INTERVAL_SECONDS);
                            self.print_stats();
                            timer.reset();
                        }
                        Token(_) => {
                            panic!("Unknown token in poll.");
                        }
                    }
                }
            }
        }
    }

}

fn main() {
    let mut rpc_server = RpcServer::new(13265);
    rpc_server.io_loop();
}
