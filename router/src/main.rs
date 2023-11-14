extern crate pretty_env_logger;
#[macro_use]
extern crate log;

use actix_web::{
    dev::PeerAddr, error, middleware, web, App, Error, HttpRequest, HttpResponse, HttpServer,
};
use awc::Client;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::hash::Hasher;
use std::net::ToSocketAddrs;
use std::sync::RwLock;
use std::time::{Duration, Instant};
use url::Url;

/// Router / load balancer
/// Built off of https://github.com/actix/examples/tree/master/http-proxy
/// Example code was pruned and then additional functionality added for
/// load balancing, static/dynamic routing, etc.

enum RoutingStrategy {
    Hash,
    MovingAverage,
}

const AVERAGE_WINDOW: Duration = Duration::from_secs(2);

struct ResponseTime {
    start: Instant,
    wait: Duration,
}

fn pick_host_from_hash<T: Hash, V>(url: &Vec<V>, key: T) -> usize {
    let mut s = DefaultHasher::new();
    key.hash(&mut s);
    (s.finish() % (url.len() as u64)) as usize
}

fn pick_host_lowest_average(averages: &Vec<RwLock<Duration>>) -> usize {
    let mut lowest = Duration::MAX;
    let mut index = usize::MAX;
    for i in 0..averages.len() {
        {
            // Acquire read lock
            let average = averages.get(i).unwrap().read().unwrap();
            if *average < lowest {
                lowest = *average;
                index = i;
            }
            // Implicit drop
        }
    }
    if index == usize::MAX {
        let mut s = DefaultHasher::new();
        Instant::now().hash(&mut s);
        return (s.finish() % (averages.len() as u64)) as usize;
    }
    index
}

/// Forwards the incoming HTTP request using `awc` (actix web client).
async fn forward(
    req: HttpRequest,
    payload: web::Payload,
    peer_addr: Option<PeerAddr>,
    client: web::Data<Client>,
    hosts: web::Data<Vec<Url>>,
    pool_count: web::Data<RwLock<isize>>,
    times: web::Data<Vec<RwLock<VecDeque<ResponseTime>>>>,
    averages: web::Data<Vec<RwLock<Duration>>>,
    agent_requests: web::Data<RwLock<HashMap<String, RwLock<VecDeque<Instant>>>>>,
) -> Result<HttpResponse, Error> {
    // TODO: parse the agent from the request body/headers
    let agent = "Test123";
    info!("Agent {agent} requested a host...");

    let mut routing_strategy = RoutingStrategy::MovingAverage;

    // Get read lock, we transform it into a write lock if needed
    let agent_map = (**agent_requests).read().unwrap();
    // New agent condition, make into write lock to add vecdeque
    if agent_map.get(agent).is_none() {
        info!("    Agent info was not found...");
        drop(agent_map);
        let mut agent_map = (**agent_requests).write().unwrap();
        agent_map.insert(String::from(agent), RwLock::new(VecDeque::new()));
        // Implicit drop of `agent_map`
    } else {
        let agent_times = agent_map.get(agent).unwrap().read().unwrap();
        info!(
            "    Agent info was found with {} requests in the window...",
            agent_times.len()
        );
        let agent_total = agent_times.len();
        drop(agent_times);
        drop(agent_map);
        // Acquire lock
        let pool = (**pool_count).read().unwrap();
        let traffic_total = *pool;
        drop(pool);
        // Calculate traffic share:
        let threshhold = (traffic_total as f32) / ((**hosts).len() as f32);
        info!("    Traffic threshhold is {}...", threshhold);

        if agent_total as f32 > threshhold {
            warn!("    Swapping {agent} to hash strategy...");
            routing_strategy = RoutingStrategy::Hash;
        }
    }

    let target_index = match routing_strategy {
        RoutingStrategy::Hash => pick_host_from_hash(&(**hosts), agent),
        RoutingStrategy::MovingAverage => pick_host_lowest_average(&(**averages)),
    };

    let mut target_url = (**hosts).get(target_index).unwrap().clone();
    info!("    Target host: {}...", target_url);

    target_url.set_path(req.uri().path());
    target_url.set_query(req.uri().query());

    let forwarded_req = client
        .request_from(target_url.as_str(), req.head())
        .no_decompress();

    // Probably unnecessary addition: just extra logging info for the coffee app hosts
    let forwarded_req = match peer_addr {
        Some(PeerAddr(addr)) => {
            forwarded_req.append_header(("forwarded", format!("for={}", addr.ip().to_string())))
        }
        None => forwarded_req,
    };

    let request_start = Instant::now();

    // match tokio::time::timeout(Duration::from_millis(10), work).await {
    //     Ok(result) => match result {
    //         Ok(response) => println!("Status: {}", response.status()),
    //         Err(e) => println!("Network error: {:?}", e),
    //     },
    //     Err(_) => println!("Timeout: no response in 10 milliseconds."),
    // };

    let res = forwarded_req
        .send_stream(payload)
        .await
        .map_err(error::ErrorInternalServerError)?;

    let request_time = ResponseTime {
        start: request_start,
        wait: request_start.elapsed(),
    };
    info!(
        "    Reached target in: {}ms...",
        request_time.wait.as_millis()
    );

    let mut net: isize = 1; // Start at 1 for first add
    let mut new_average: Duration = Duration::ZERO;
    {
        // Acquire lock
        let mut host_times = (**times).get(target_index).unwrap().write().unwrap();
        // Add new time
        host_times.push_back(request_time);
        // Drop old times
        for i in 0..host_times.len() {
            if host_times
                .get(i)
                .unwrap()
                .start
                .duration_since(request_start)
                > AVERAGE_WINDOW
            {
                break;
            }
            host_times.pop_front();
            net -= 1;
        }
        for i in 0..host_times.len() {
            new_average += host_times.get(i).unwrap().wait;
        }
        new_average = match host_times.len() == 0 {
            true => Duration::ZERO,
            false => new_average / host_times.len() as u32,
        };
        // Implicit drop
    }
    {
        // Acquire lock
        let mut average = (**averages).get(target_index).unwrap().write().unwrap();
        *average = new_average;
        // Implicit drop
    }
    {
        // Acquire lock
        let mut count = (**pool_count).write().unwrap();
        *count += net;
        // Implicit drop
    }
    info!("    hI?...");
    {
        // Acquire lock
        let agent_map = (**agent_requests).read().unwrap();
        let mut agent_times = agent_map.get(agent).unwrap().write().unwrap();
        // Drop old times
        info!("    Remapping agent history...");
        for i in 0..agent_times.len() {
            info!(
                "        {}ms request...",
                agent_times
                    .get(i)
                    .unwrap()
                    .duration_since(request_start)
                    .as_millis()
            );
            if agent_times.get(i).unwrap().duration_since(request_start) > AVERAGE_WINDOW {
                info!("        Over {} -- dropped!", AVERAGE_WINDOW.as_millis());
                break;
            }
            agent_times.pop_front();
        }
        // Add new time
        agent_times.push_back(request_start);
        // Implicit drop
    }

    let mut client_resp = HttpResponse::build(res.status());
    // Remove `Connection` as per
    // https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Connection#Directives
    for (header_name, header_value) in res.headers().iter().filter(|(h, _)| *h != "connection") {
        client_resp.insert_header((header_name.clone(), header_value.clone()));
    }

    Ok(client_resp.streaming(res))
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    pretty_env_logger::init();
    fn get_host(url: &str, port: u16) -> Url {
        let forward_socket_addr = (url, port)
            .to_socket_addrs()
            .unwrap()
            .next()
            .expect("given forwarding address was not valid");
        // TODO: support https
        let forward_url = format!("http://{forward_socket_addr}");
        Url::parse(&forward_url).unwrap()
    }
    HttpServer::new(move || {
        let hosts = vec![
            get_host("127.0.0.1", 3000),
            get_host("127.0.0.1", 3001),
            get_host("127.0.0.1", 3002),
        ];
        let pool_count = RwLock::new(0 as isize);
        let mut times: Vec<RwLock<VecDeque<ResponseTime>>> = Vec::new();
        let mut averages: Vec<RwLock<Duration>> = Vec::new();
        for _ in 0..hosts.len() {
            times.push(RwLock::new(VecDeque::new()));
            averages.push(RwLock::new(Duration::ZERO));
        }
        let agent_requests: RwLock<HashMap<String, RwLock<VecDeque<Instant>>>> =
            RwLock::new(HashMap::new());
        App::new()
            .app_data(web::Data::new(Client::default()))
            .app_data(web::Data::new(hosts))
            .app_data(web::Data::new(pool_count))
            .app_data(web::Data::new(times))
            .app_data(web::Data::new(averages))
            .app_data(web::Data::new(agent_requests))
            .wrap(middleware::Logger::default())
            .default_service(web::to(forward))
    })
    .bind(("127.0.0.1", 8080))?
    .workers(2)
    .run()
    .await
}
