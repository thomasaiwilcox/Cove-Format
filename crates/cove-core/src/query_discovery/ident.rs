pub(super) fn table_root(name: &str) -> String {
    format!("table({})", coveql_identifier(name))
}

pub(super) fn object_root(type_name: &str) -> String {
    format!("object({})", coveql_identifier(type_name))
}

pub(super) fn projection_root(projection_id: &str) -> String {
    format!("projection({})", coveql_identifier(projection_id))
}

pub fn coveql_identifier(name: &str) -> String {
    if is_plain_coveql_identifier(name) && !is_reserved_coveql_identifier(name) {
        return name.to_string();
    }
    let escaped = name.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn is_plain_coveql_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return false;
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn is_reserved_coveql_identifier(name: &str) -> bool {
    matches!(
        name,
        "and"
            | "as"
            | "association"
            | "edge"
            | "evidence"
            | "explain"
            | "false"
            | "from"
            | "graph"
            | "group"
            | "in"
            | "node"
            | "not"
            | "null"
            | "object"
            | "or"
            | "order"
            | "projection"
            | "select"
            | "table"
            | "true"
            | "where"
    )
}
