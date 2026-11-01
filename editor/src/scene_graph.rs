/// The pointer of the module a pointer into a file is within: an entry of a `modules` list, or a
/// node of a scene graph.
pub fn module_pointer(pointer: &str) -> Option<String> {
    if let Some(rest) = pointer.strip_prefix("/modules/") {
        let index = rest.split('/').next()?;
        index.parse::<usize>().ok()?;
        return Some(format!("/modules/{index}"));
    }
    let node = node_pointer(pointer);
    (!node.is_empty()).then_some(node)
}

/// The pointer of the scene node a pointer into a scene graph is within, through its folders.
pub(crate) fn node_pointer(pointer: &str) -> String {
    let segments: Vec<&str> = pointer.split('/').skip(1).collect();
    let mut node = String::new();
    let mut at = 0;
    while let Some(index) = segments.get(at).filter(|s| s.parse::<usize>().is_ok()) {
        node.push('/');
        node.push_str(index);
        if segments.get(at + 1) != Some(&"children")
            || segments
                .get(at + 2)
                .is_none_or(|s| s.parse::<usize>().is_err())
        {
            break;
        }
        node.push_str("/children");
        at += 2;
    }
    node
}
