//! Strict, Record-bound protocol carried by the existing same-user GUI socket.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    pub(crate) protocol: u8,
    pub(crate) id: String,
    pub(crate) owner: String,
    pub(crate) operation: String,
    pub(crate) deadline: u64,
    #[serde(default)]
    pub(crate) expected: Option<Expected>,
    #[serde(default, rename = "permissionId")]
    pub(crate) permission_id: Option<String>,
    #[serde(default)]
    pub(crate) key: Option<String>,
    #[serde(default)]
    pub(crate) modifiers: Option<u32>,
    #[serde(default)]
    pub(crate) shortcut: Option<Shortcut>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Expected {
    #[serde(rename = "processEpoch")]
    pub(crate) process_epoch: String,
    pub(crate) revision: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Shortcut {
    pub(crate) key_code: u32,
    pub(crate) modifiers: u32,
    pub(crate) label: String,
}

#[derive(Clone, Debug)]
pub(crate) enum Operation {
    ReadSnapshot,
    RequestPermission(String),
    OpenSystemSettings(String),
    ValidateRecordShortcut(String, u32),
    SaveRecordShortcut(Shortcut),
}

#[derive(Clone, Debug)]
pub(crate) struct Admitted {
    pub(crate) id: String,
    pub(crate) deadline: u64,
    pub(crate) expected: Option<Expected>,
    pub(crate) operation: Operation,
}

const PERMISSIONS: &[&str] = &[
    "screen-recording",
    "microphone",
    "accessibility",
    "camera",
    "speech-recognition",
];

pub(crate) fn parse(line: &str, now: u64) -> Option<Admitted> {
    if line.len() > 4096 {
        return None;
    }
    let request: Request = serde_json::from_str(line).ok()?;
    if request.protocol != 1
        || request.owner != "record"
        || request.id.len() > 64
        || request.id.is_empty()
        || !request
            .id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        || request.deadline <= now
        || request.deadline > now.saturating_add(3000)
    {
        return None;
    }
    let operation = match request.operation.as_str() {
        "readSnapshot"
            if request.expected.is_none()
                && request.permission_id.is_none()
                && request.key.is_none()
                && request.modifiers.is_none()
                && request.shortcut.is_none() =>
        {
            Operation::ReadSnapshot
        }
        "requestPermission"
            if request.expected.is_some()
                && request
                    .permission_id
                    .as_deref()
                    .is_some_and(valid_permission)
                && request.key.is_none()
                && request.modifiers.is_none()
                && request.shortcut.is_none() =>
        {
            Operation::RequestPermission(request.permission_id.clone()?)
        }
        "openSystemSettings"
            if request.expected.is_some()
                && request
                    .permission_id
                    .as_deref()
                    .is_some_and(valid_permission)
                && request.key.is_none()
                && request.modifiers.is_none()
                && request.shortcut.is_none() =>
        {
            Operation::OpenSystemSettings(request.permission_id.clone()?)
        }
        "validateRecordShortcut"
            if request.expected.is_some()
                && request.key.as_deref().is_some_and(valid_key)
                && request.modifiers.is_some_and(|value| value <= 0xffff)
                && request.permission_id.is_none()
                && request.shortcut.is_none() =>
        {
            Operation::ValidateRecordShortcut(request.key.clone()?, request.modifiers?)
        }
        "saveRecordShortcut"
            if request.expected.is_some()
                && request.shortcut.as_ref().is_some_and(valid_shortcut)
                && request.permission_id.is_none()
                && request.key.is_none()
                && request.modifiers.is_none() =>
        {
            Operation::SaveRecordShortcut(request.shortcut.clone()?)
        }
        _ => return None,
    };
    if request.expected.as_ref().is_some_and(|expected| {
        expected.process_epoch.is_empty() || expected.process_epoch.len() > 64
    }) {
        return None;
    }
    Some(Admitted {
        id: request.id,
        deadline: request.deadline,
        expected: request.expected,
        operation,
    })
}

fn valid_permission(value: &str) -> bool {
    PERMISSIONS.contains(&value)
}
fn valid_key(value: &str) -> bool {
    !value.is_empty() && value.len() <= 16 && value.bytes().all(|b| b.is_ascii_alphanumeric())
}
fn valid_shortcut(value: &Shortcut) -> bool {
    value.key_code <= 255
        && value.modifiers <= 0xffff
        && !value.label.is_empty()
        && value.label.len() <= 32
}

#[cfg(test)]
mod tests {
    use super::*;
    fn req(operation: &str, extra: &str) -> String {
        format!(
            r#"{{"protocol":1,"id":"req-1","owner":"record","operation":"{operation}","deadline":3000{extra}}}"#
        )
    }
    #[test]
    fn protocol_is_versioned_correlated_owner_bound_and_strict() {
        assert!(matches!(
            parse(&req("readSnapshot", ""), 1000).unwrap().operation,
            Operation::ReadSnapshot
        ));
        for line in [
            req("readSnapshot", r#","path":"/tmp""#),
            req("readSnapshot", r#","owner":"desktop""#),
            req("readSnapshot", r#","protocol":2"#),
            req(
                "requestPermission",
                r#","expected":{"processEpoch":"e","revision":1},"permissionId":"unknown""#,
            ),
            req(
                "validateRecordShortcut",
                r#","expected":{"processEpoch":"e","revision":1},"key":"a","modifiers":999999"#,
            ),
        ] {
            assert!(parse(&line, 1000).is_none(), "{line}");
        }
        assert!(parse(&req("readSnapshot", ""), 3000).is_none());
    }
}
