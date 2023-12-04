extern crate pretty_env_logger;
#[macro_use]
extern crate log;

use actix_web::{
    dev::PeerAddr, error, middleware, web, App, Error, HttpRequest, HttpResponse, HttpServer,
};
use awc::Client;
use clap::Parser;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::hash::Hasher;
use std::net::ToSocketAddrs;
use std::sync::RwLock;
use std::time::{Duration, Instant};
use tokio::time::timeout;
use url::Url;

/// Router / load balancer
/// Built off of https://github.com/actix/examples/tree/master/http-proxy (Licensed under APACHE 2.0)
/// Example code was pruned and then additional functionality added for
/// load balancing, static/dynamic routing, etc.

enum RoutingStrategy {
    Hash,
    MovingAverage,
}

const HOST_FILE: &str = "hosts.yaml";
const AVERAGE_WINDOW: Duration = Duration::from_secs(2);
const GRACE_REQUESTS: usize = 1;
const TIMEOUT_THRESHHOLD: Duration = Duration::from_millis(500);
const TIMEOUT_PENALTY: Duration = Duration::from_millis(500);

struct ResponseInfo {
    start: Instant,
    wait: Option<Duration>,
}

fn pick_host_from_hash<T: Hash, V>(urls: &Vec<V>, key: T, exclude_indexes: &Vec<usize>) -> usize {
    // Uses simple linear probing
    fn probe(i: usize) -> usize {
        i + 1
    }
    assert!(
        urls.len() > exclude_indexes.len(),
        "Cannot exclude all urls from a host pick!"
    );
    let mut s = DefaultHasher::new();
    key.hash(&mut s);
    let mut out = (s.finish() % (urls.len() as u64)) as usize;
    while exclude_indexes.contains(&out) {
        out = probe(out) % urls.len();
    }
    out
}

fn pick_host_lowest_average(
    averages: &Vec<RwLock<Duration>>,
    exclude_indexes: &Vec<usize>,
) -> usize {
    // Uses simple linear probing
    fn probe(i: usize) -> usize {
        i + 1
    }
    let mut lowest = Duration::MAX;
    let mut out = usize::MAX;
    for i in 0..averages.len() {
        {
            // Acquire read lock
            let average = averages.get(i).unwrap().read().unwrap();
            if *average < lowest {
                lowest = *average;
                out = i;
            }
            // Implicit drop
        }
    }
    if out == usize::MAX {
        let mut s = DefaultHasher::new();
        Instant::now().hash(&mut s);
        out = (s.finish() % (averages.len() as u64)) as usize;
    }
    while exclude_indexes.contains(&out) {
        out = probe(out) % averages.len();
    }
    out
}

/// Forwards the incoming HTTP request using `awc` (actix web client).
async fn forward(
    req: HttpRequest,
    payload: web::Payload,
    peer_addr: Option<PeerAddr>,
    client: web::Data<Client>,
    hosts: web::Data<Vec<Url>>,
    pool_count: web::Data<RwLock<isize>>,
    times: web::Data<Vec<RwLock<VecDeque<ResponseInfo>>>>,
    averages: web::Data<Vec<RwLock<Duration>>>,
    agent_requests: web::Data<RwLock<HashMap<String, RwLock<VecDeque<Instant>>>>>,
) -> Result<HttpResponse, Error> {
    // TODO: parse the agent from the request body/headers
    let agent = String::from(match req.peer_addr() {
        Some(v) => String::from(v.ip().to_string()),
        None => String::from("anonymous"),
    });
    info!("Received request from user agent \"{agent}\":");
    let routing_strategy = pick_strategy(&agent, &agent_requests, &pool_count, &hosts);
    let excludes: Vec<usize> = Vec::new();

    forward_request(
        routing_strategy,
        &excludes,
        hosts,
        &agent,
        averages,
        req,
        client,
        peer_addr,
        payload,
        times,
        pool_count,
        agent_requests,
    )
    .await
}

async fn forward_request(
    routing_strategy: RoutingStrategy,
    exclude_hosts: &Vec<usize>,
    hosts: web::Data<Vec<Url>>,
    agent: &str,
    averages: web::Data<Vec<RwLock<Duration>>>,
    req: HttpRequest,
    client: web::Data<Client>,
    peer_addr: Option<PeerAddr>,
    payload: web::Payload,
    times: web::Data<Vec<RwLock<VecDeque<ResponseInfo>>>>,
    pool_count: web::Data<RwLock<isize>>,
    agent_requests: web::Data<RwLock<HashMap<String, RwLock<VecDeque<Instant>>>>>,
) -> Result<HttpResponse, Error> {
    let target_index = match routing_strategy {
        RoutingStrategy::Hash => pick_host_from_hash(&(**hosts), agent, exclude_hosts),
        RoutingStrategy::MovingAverage => pick_host_lowest_average(&(**averages), exclude_hosts),
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

    let req = forwarded_req.send_stream(payload);

    let res = match timeout(TIMEOUT_THRESHHOLD, req).await {
        Ok(result) => result.map_err(error::ErrorInternalServerError),
        Err(_) => Err(error::ErrorGatewayTimeout(
            "Failed to reach host before timeout",
        )),
    };

    let request_time = ResponseInfo {
        start: request_start,
        wait: Some(request_start.elapsed()),
    };
    match request_time.wait {
        Some(v) => info!("    Reached target in: {}ms...", v.as_millis()),
        None => warn!(
            "    Failed to reach target! Timeout at: {}ms...",
            TIMEOUT_THRESHHOLD.as_millis()
        ),
    };

    update_load_state(
        times,
        target_index,
        request_time,
        request_start,
        averages,
        pool_count,
        agent_requests,
        agent,
    );

    let res = res?;
    let mut client_resp = HttpResponse::build(res.status());
    // Remove `Connection` as per
    // https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Connection#Directives
    for (header_name, header_value) in res.headers().iter().filter(|(h, _)| *h != "connection") {
        client_resp.insert_header((header_name.clone(), header_value.clone()));
    }

    Ok(client_resp.streaming(res))
}

fn update_load_state(
    times: web::Data<Vec<RwLock<VecDeque<ResponseInfo>>>>,
    target_index: usize,
    request_time: ResponseInfo,
    request_start: Instant,
    averages: web::Data<Vec<RwLock<Duration>>>,
    pool_count: web::Data<RwLock<isize>>,
    agent_requests: web::Data<RwLock<HashMap<String, RwLock<VecDeque<Instant>>>>>,
    agent: &str,
) {
    let mut net: isize = 1;
    // Start at 1 for first add
    let mut new_average: Duration = Duration::ZERO;
    {
        // Acquire lock
        let mut host_times = (**times).get(target_index).unwrap().write().unwrap();
        // Add new time
        host_times.push_back(request_time);
        // Drop old times
        let mut i = 0;
        while i < host_times.len() {
            if request_start.duration_since(host_times.get(i).unwrap().start) < AVERAGE_WINDOW {
                break;
            }
            host_times.pop_front();
            net -= 1;
            i += 1;
        }
        for i in 0..host_times.len() {
            new_average += match host_times.get(i).unwrap().wait {
                Some(v) => v,
                None => TIMEOUT_PENALTY,
            };
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
    {
        // Acquire lock
        let agent_map = (**agent_requests).read().unwrap();
        let mut agent_times = agent_map.get(agent).unwrap().write().unwrap();
        // Drop old times
        info!("    Remapping agent history...");
        let mut i = 0;
        while i < agent_times.len() {
            info!(
                "        {}ms request...",
                request_start
                    .duration_since(*agent_times.get(i).unwrap())
                    .as_millis()
            );
            if request_start.duration_since(*agent_times.get(i).unwrap()) < AVERAGE_WINDOW {
                break;
            }
            info!("        Over {} -- dropped!", AVERAGE_WINDOW.as_millis());
            agent_times.pop_front();
            i += 1;
        }
        // Add new time
        agent_times.push_back(request_start);
        // Implicit drop
    }
}

fn pick_strategy(
    agent: &str,
    agent_requests: &web::Data<RwLock<HashMap<String, RwLock<VecDeque<Instant>>>>>,
    pool_count: &web::Data<RwLock<isize>>,
    hosts: &web::Data<Vec<Url>>,
) -> RoutingStrategy {
    info!("Agent {agent} requested a host...");

    let mut routing_strategy = RoutingStrategy::MovingAverage;

    // Get read lock, we transform it into a write lock if needed
    let agent_map = (***agent_requests).read().unwrap();
    // New agent condition, make into write lock to add vecdeque
    if agent_map.get(agent).is_none() {
        info!("    Agent info was not found...");
        drop(agent_map);
        let mut agent_map = (***agent_requests).write().unwrap();
        agent_map.insert(String::from(agent), RwLock::new(VecDeque::new()));
        // Implicit drop of `agent_map`
    } else {
        let agent_times = agent_map.get(agent).unwrap().read().unwrap();
        info!(
            "    Agent info was found with {} requests in the window...",
            agent_times.len()
        );
        let agent_total = agent_times.len() - GRACE_REQUESTS;
        drop(agent_times);
        drop(agent_map);
        // Acquire lock
        let pool = (***pool_count).read().unwrap();
        let traffic_total = *pool;
        drop(pool);
        // Calculate traffic share:
        let threshhold = (traffic_total as f32) / ((***hosts).len() as f32);
        info!("    Traffic threshhold is {}...", threshhold);

        if agent_total as f32 > threshhold {
            warn!("    Swapping {agent} to hash strategy...");
            routing_strategy = RoutingStrategy::Hash;
        }
    }
    routing_strategy
}

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Port to receive HTTP traffic on
    #[arg(short, long, default_value_t = 80)]
    receive_port: u16,
    /// Port to send HTTP traffic to
    #[arg(short, long, default_value_t = 80)]
    send_port: u16,
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    pretty_env_logger::init();
    let args = Args::parse();
    let f = std::fs::File::open(HOST_FILE)?;
    let data: serde_yaml::Value = serde_yaml::from_reader(f)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::NotFound, err))?;
    let yaml_hosts = data["apphost"]["hosts"]
        .as_mapping()
        .ok_or(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Failed to parse mapping from {}", HOST_FILE),
        ))?;
    let mut hosts: Vec<Url> = Vec::new();
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
    for host_tuple in yaml_hosts.iter() {
        hosts.push(get_host(
            host_tuple.0.as_str().ok_or(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Failed to parse mapping from {}", HOST_FILE),
            ))?,
            args.send_port,
        ));
        info!(
            "Found new app host {}",
            hosts.get(hosts.len() - 1).unwrap().to_string()
        );
    }
    HttpServer::new(move || {
        let pool_count = RwLock::new(0 as isize);
        let mut times: Vec<RwLock<VecDeque<ResponseInfo>>> = Vec::new();
        let mut averages: Vec<RwLock<Duration>> = Vec::new();
        for _ in 0..hosts.len() {
            times.push(RwLock::new(VecDeque::new()));
            averages.push(RwLock::new(Duration::ZERO));
        }
        let agent_requests: RwLock<HashMap<String, RwLock<VecDeque<Instant>>>> =
            RwLock::new(HashMap::new());
        App::new()
            .app_data(web::Data::new(Client::default()))
            .app_data(web::Data::new(hosts.clone()))
            .app_data(web::Data::new(pool_count))
            .app_data(web::Data::new(times))
            .app_data(web::Data::new(averages))
            .app_data(web::Data::new(agent_requests))
            .wrap(middleware::Logger::default())
            .default_service(web::to(forward))
    })
    .bind(("127.0.0.1", args.receive_port))?
    .run()
    .await
}
