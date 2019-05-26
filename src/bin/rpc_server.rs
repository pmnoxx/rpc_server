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
use redis::{Client, Connection};
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


struct RedisModel {
    client : Client

}

impl RedisModel {
    pub fn new() -> RedisModel {
        let client = redis::Client::open("redis://127.0.0.1/").unwrap();
        RedisModel {
            client
        }
    }
    pub fn get_connection(&mut self) -> Connection {
        return self.client.get_connection().unwrap();
    }

    pub fn do_some_redis_op(&mut self) -> i64 {
        // connect to redis
        let con = self.get_connection();
        // throw away the result, just make sure it does not fail
        for _x in 1..1000000 {
            let _: () = con.set("my_key", _x).unwrap();
        }
        // read back the key and return it.  Because the return value
        // from the function is a result for integer this will automatically
        // convert into one.
        return con.get("my_key").unwrap();
    }
}


struct Handler {
    redis_model : RedisModel
}

impl Handler {
    pub fn new() -> Handler {
        Handler {
            redis_model : RedisModel::new()
        }
    }

    pub fn fetch_an_integer(&mut self) -> i64 {
        return self.redis_model.do_some_redis_op();
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

        let addr2 = format!("{}:{}", "127.0.0.1", (self.port + 1).to_string()).parse().unwrap();
        let server2 = TcpListener::bind(&addr2).unwrap();
        self.poll.register(&server2, Token(2), Ready::readable(), PollOpt::level())
            .unwrap();
        println!("Listening on port {}", self.port);

        // add timer example
        let timer = PeriodicTimer::new(TIMER_INTERVAL_SECONDS);
        self.poll.register(&timer, Token(1), Ready::readable(), PollOpt::level())
            .unwrap();

        let mut handler = Handler::new();

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

                    // println!("fetch {}", fetch_an_integer().unwrap());

                    let mut len;
                    {
                        let mut stream = self.connections.get(&event.token()).unwrap();

                        let mut buf = Vec::new();
                        let _response = stream.read_to_end(&mut buf);
                        len = buf.len();
                        print!("{}: ", len);
                        if len >= 4 {
                            if buf[0] == 65 {
                                println!("fetched {}", handler.fetch_an_integer());
                            }
                        }
                        for val in buf {
                            print!("{} ", val);
                        }
                        println!("\n");
                        if len == 0 {
                            println!("Connection closed for token: {}", event.token().0);
                            self.poll.deregister(stream).unwrap();
                        }
                    }
                    if len == 0 {
                        self.connections.remove_entry(&event.token()).unwrap();
                    }
                    self.num_rpc_count += 1;
                } else {
                    match event.token() {
                        Token(0) => {
                            // new connection
                            let result = server.accept();
                            println!("got tcp connection");
                            let (mut stream, _s2) = result.unwrap();
                            stream.write(b"hello\n").unwrap();

                            self.poll.register(&stream, Token(self.connection_id), Ready::readable(), PollOpt::level()).unwrap();

                            self.connections.insert(Token(self.connection_id), stream);
                            self.connection_id += 1;

                            // {"headers": {"support_msgpack_prepend": true}, "data": {"name": "Maintenance.264"}, "method": "name_channel"}
                        }
                        Token(2) => {
                            // new connection
                            let result = server2.accept();
                            println!("got tcp connection");
                            let (mut stream, _s2) = result.unwrap();
                            stream.write(b"hello\n").unwrap();

                            self.poll.register(&stream, Token(self.connection_id), Ready::readable(), PollOpt::level()).unwrap();

                            self.connections.insert(Token(self.connection_id), stream);
                            self.connection_id += 1;
                        }
                        Token(1) => {
                            println!("{}-second timer", TIMER_INTERVAL_SECONDS);
                            self.print_stats();
                            timer.reset();
                        }
                        Token(_) => {
                            panic!("Unknown token in poll. {}", event.token().0);
                        }
                    }
                }
            }
        }
    }

}

fn main() {
    let mut rpc_server = RpcServer::new(5004);
    rpc_server.io_loop();
}
