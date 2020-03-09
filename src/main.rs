use std::collections::HashMap;
use std::env;
use std::fmt;
use std::process::Command;
use std::str;
use std::sync::mpsc;
use std::thread;
use crossterm::style::{style, Color, Attribute};

// Represents a mode that the node is in. Theoretically there are only to modes: leader and follower. 
// But since we only get a string from the server we can't really be sure if there's no error, 
// or some new mode has been introduced - that's why Unknown exists.
//
// On the other hand a Leader is a special node that returns some specific information. 
// That's why we need to able to distinguish between them in the first place.
#[derive(Clone, Copy, Debug)]
enum Mode {
    Follower,
    Leader,
    Standalone,
    Unknown,
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

struct KafkaCluster {
    ids: Vec<String>,
}

struct ZNode {
    id: String,
    mode: Mode,
}

struct ZookeeperClient {
    host: String,
    port: String,
}

impl ZookeeperClient {

    fn new(host: String, port: String) -> ZookeeperClient {
        ZookeeperClient {
            host: host,
            port: port,
        }
    }

    fn call_nc(&self, command: &String, grep: &String) -> Option<String> {
        let com = format!("echo -n '{}' | nc -w 5 {} {} | grep {}", command, self.host, self.port, grep);
    
        let output = Command::new("sh")
            .arg("-c")
            .arg(com)
            .output()
            .expect("no connection");
    
        let output_status = output.status;
    
        if output_status.success() {
            let mut output_std: Vec<u8> = output.stdout.clone();
            output_std.truncate(output_std.len() - 1); //remove trailing whitespace
            let pref_len = grep.len();
            let output_std_f: Vec<u8> = output_std.drain(pref_len+1..).collect();
            let output_str = str::from_utf8(&output_std_f).unwrap();
            let output_str_f = ZookeeperClient::remove_first(output_str).unwrap_or("");

            
            return Some(output_str.trim().to_string());
        } else {
            return None;
        }
    }

    fn remove_first(s: &str) -> Option<&str> {
        s.chars().next().map(|c| &s[c.len_utf8()..])
    }

    fn get_status(&self) -> Option<ZNode> {
        let mode = self.call_nc(&"srvr".to_string(), &"Mode".to_string());
        let server_id = self.call_nc(&"conf".to_string(), &"serverId".to_string());

        let znode: Option<ZNode> = match (mode, server_id) {
            (Some(m), Some(id)) => {
                let mode = match m.as_str() {
                    "follower"   => Mode::Follower,
                    "leader"     => Mode::Leader,
                    "standalone" => Mode::Standalone,
                    _ => Mode::Unknown,
                };

                let znode = ZNode {
                    id: id,
                    mode: mode
                };

                Some(znode)
            },
            _ => None
        };

        znode
    }

    fn get_brokers(&self) -> Option<KafkaCluster> {
        let brokers = self.call_nc(&"wchc".to_string(), &"/brokers/ids".to_string());
        brokers.map(|ls| KafkaCluster {
            ids: ls.lines().map(|x| x.to_string()).collect()
        })
    }
}

struct ZkEnsembleService {
    nodes: Vec<(String, String)>
}

impl ZkEnsembleService {
    
    fn new(nodes: Vec<(String, String)>) -> ZkEnsembleService {
        ZkEnsembleService {
            nodes: nodes,
        }
    }
}

/// Splits a String of format `host:port` into a tuple.
fn split(s: &String) -> (String, String) {
    let pair_vec: Vec<String> = s.split(':').map(|s| s.to_string()).collect();
    
    if pair_vec.len() == 2 {
        (pair_vec[0].clone(), pair_vec[1].clone()) //probably better not to clone
    } else {
        panic!("Wrong parameter format. Should be 'host:port'");
    }
}

/// Returns length of the longest String in this Vector.
fn max_len(v: &Vec<&String>) -> usize {
    let max_host = v.iter().fold(v[0], |acc, &t| {
        if t.len() > acc.len() {
            t
        } else {
            acc
        }
    });

    return max_host.len();
}

fn main() {
    println!("Zookeeper ensemble status:");

    let args: Vec<String> = env::args().collect::<Vec<String>>().drain(1..).collect(); //drop the first arg
    let args_iter = args.iter();
    let args_split: Vec<(String, String)> = args_iter.map(|arg| split(&arg.to_string())).collect();
    let hosts: Vec<&String> = args_split.iter().map(|arg| &arg.0).collect();
    let max_host_len = max_len(&hosts);

    let hosts_size: usize = hosts.len();

    let (tx, rx) = mpsc::channel();

    let mut threads = vec![];

    let mut status = HashMap::new();

    for (host, port) in &args_split {

        let txc = mpsc::Sender::clone(&tx);

        let h = host.clone();
        let p = port.clone();

        threads.push(thread::spawn(move || {
            let client = ZookeeperClient::new(h.clone(), p);
            let znode = client.get_status();
            txc.send((h, znode));
        }));

    }

    for (h, znode) in rx {        
        status.insert(h, znode);

        let i = status.len();
        if  i == hosts_size {
            break;
        }
    }

    for thread in threads {
        let _ = thread.join();
    }

    //keep the hosts ordering from the original parameter list
    for h in hosts {
        let znode = status.get(h.as_str()).unwrap();
        let (id, mode) = znode.as_ref().map_or_else(|| ("_".to_string(), "no connection".to_string()), |zn| (zn.id.clone(), zn.mode.to_string()));
        let color = znode.as_ref().map_or_else(|| Color::Blue, |zn| match zn.mode {
            Mode::Follower => Color::Cyan,
            Mode::Leader   => Color::Magenta,
            Mode::Standalone => Color::Yellow,
            Mode::Unknown  => Color::Red,
        });
        let styled_id = style(id)
            .with(Color::Yellow)
            .attribute(Attribute::Bold);
        let styled_mode = style(mode)
            .with(color)
            .attribute(Attribute::Bold);
        println!("{}", format!("{}: {:width$} : {}", styled_id, h, styled_mode, width = max_host_len));
    }

}
