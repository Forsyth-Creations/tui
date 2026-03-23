use auto_discovery::Node;


fn main(){
    // Tell everyone I am here using mdns!
    let mut node = Node::new("davinci".to_string());
    node.passcode("1234").broadcast_existence();

}