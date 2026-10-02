// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! The JSON request envelope that the Node and C# SDKs send through
//! `mxc_ffi`: `{"tools": <input | input[]>, "context"?: {...}}`.
//!
//! A tool input is a bare invocation name or
//! `{"invocationName", "packageUrl"?, "detectedVersion"?, "intent"?}`. The context keys
//! are the camelCase `ResolveContext` fields. An explicit `null` is the same
//! as an omitted optional field. Unknown keys and wrongly typed values are
//! `malformed_request` (`invalid_context`) failures.

use crate::errors::{invalid_context, Result};
use crate::json::{Json, JsonObject};
use crate::model::{ResolveContext, SymbolMap, ToolCandidate, ToolInput, ToolInputs};

fn only_keys(object: &JsonObject, allowed: &[&str], at: &str) -> Result<()> {
    match object.keys().find(|key| !allowed.contains(key)) {
        Some(key) => Err(invalid_context(format!(
            "{at}.{key} is not a supported field"
        ))),
        None => Ok(()),
    }
}

fn optional_string(object: &JsonObject, key: &str, at: &str) -> Result<Option<String>> {
    match object.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(invalid_context(format!("{at}.{key} must be a string"))),
    }
}

fn tool_input(value: &Json, index: usize) -> Result<ToolInput> {
    let at = format!("tools[{index}]");
    match value {
        Json::String(name) => Ok(ToolInput::Name(name.clone())),
        Json::Object(object) => {
            only_keys(
                object,
                &["invocationName", "packageUrl", "detectedVersion", "intent"],
                &at,
            )?;
            let Some(invocation_name) = optional_string(object, "invocationName", &at)? else {
                return Err(invalid_context(format!("{at}.invocationName is required")));
            };
            Ok(ToolInput::Candidate(ToolCandidate {
                invocation_name,
                package_url: optional_string(object, "packageUrl", &at)?,
                detected_version: optional_string(object, "detectedVersion", &at)?,
                intent: optional_string(object, "intent", &at)?,
            }))
        }
        _ => Err(invalid_context(format!(
            "{at} must be an invocation name or a tool candidate object"
        ))),
    }
}

/// One tool input or an array of them.
pub fn parse_tool_inputs(value: &Json) -> Result<ToolInputs> {
    match value {
        Json::Array(items) => items
            .iter()
            .enumerate()
            .map(|(index, item)| tool_input(item, index))
            .collect::<Result<Vec<_>>>()
            .map(ToolInputs),
        single => Ok(ToolInputs(vec![tool_input(single, 0)?])),
    }
}

/// A `ResolveContext` object; `None` or `null` is the empty context.
pub fn parse_resolve_context(value: Option<&Json>) -> Result<ResolveContext> {
    let object = match value {
        None | Some(Json::Null) => return Ok(ResolveContext::new()),
        Some(Json::Object(object)) => object,
        Some(_) => return Err(invalid_context("context must be an object")),
    };
    let at = "context";
    only_keys(
        object,
        &[
            "projectRoot",
            "symbols",
            "platform",
            "architecture",
            "catalogRevision",
            "allowWeakIdentityFallback",
        ],
        at,
    )?;
    let symbols = match object.get("symbols") {
        None | Some(Json::Null) => None,
        Some(Json::Object(entries)) => {
            let mut map = SymbolMap::new();
            for (name, value) in entries.iter() {
                let Some(value) = value.as_str() else {
                    return Err(invalid_context(format!(
                        "{at}.symbols.{name} must be a string"
                    )));
                };
                map.insert(name, value);
            }
            Some(map)
        }
        Some(_) => return Err(invalid_context(format!("{at}.symbols must be an object"))),
    };
    let allow_weak_identity_fallback = match object.get("allowWeakIdentityFallback") {
        None | Some(Json::Null) => false,
        Some(Json::Bool(value)) => *value,
        Some(_) => {
            return Err(invalid_context(format!(
                "{at}.allowWeakIdentityFallback must be a boolean"
            )))
        }
    };
    Ok(ResolveContext {
        project_root: optional_string(object, "projectRoot", at)?,
        symbols,
        platform: optional_string(object, "platform", at)?,
        architecture: optional_string(object, "architecture", at)?,
        catalog_revision: optional_string(object, "catalogRevision", at)?,
        allow_weak_identity_fallback,
    })
}

/// Parses the whole `{"tools", "context"?}` request document.
pub fn parse_resolve_request(text: &str) -> Result<(ToolInputs, ResolveContext)> {
    let value = Json::parse(text)
        .map_err(|e| invalid_context(format!("request is not valid JSON: {e}")))?;
    let Some(object) = value.as_object() else {
        return Err(invalid_context("request must be an object"));
    };
    only_keys(object, &["tools", "context"], "request")?;
    let Some(tools) = object.get("tools") else {
        return Err(invalid_context("request.tools is required"));
    };
    Ok((
        parse_tool_inputs(tools)?,
        parse_resolve_context(object.get("context"))?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::ErrorReason;

    #[test]
    fn parses_a_full_request() {
        let (tools, ctx) = parse_resolve_request(
            r#"{"tools":["git",{"invocationName":"npm","packageUrl":"pkg:npm/npm","detectedVersion":null,"intent":"install"}],
                "context":{"platform":"linux","allowWeakIdentityFallback":true,"symbols":{"a":"/x"},"projectRoot":null}}"#,
        )
        .unwrap();
        assert_eq!(tools.0.len(), 2);
        assert_eq!(tools.0[0], ToolInput::Name("git".into()));
        assert_eq!(
            tools.0[1],
            ToolInput::Candidate(
                ToolCandidate::new("npm")
                    .with_package_url("pkg:npm/npm")
                    .with_intent("install")
            )
        );
        assert_eq!(ctx.platform.as_deref(), Some("linux"));
        assert!(ctx.allow_weak_identity_fallback);
        assert_eq!(ctx.symbols.unwrap().get("a"), Some("/x"));
        assert_eq!(ctx.project_root, None);
    }

    #[test]
    fn a_single_input_is_a_one_element_list() {
        let (tools, ctx) = parse_resolve_request(r#"{"tools":"git"}"#).unwrap();
        assert_eq!(tools, ToolInputs(vec!["git".into()]));
        assert_eq!(ctx, ResolveContext::new());
    }

    #[test]
    fn rejects_malformed_requests_as_invalid_context() {
        for text in [
            "not json",
            "[]",
            "{}",
            r#"{"tools":"git","extra":1}"#,
            r#"{"tools":1}"#,
            r#"{"tools":[{"packageUrl":"pkg:npm/x"}]}"#,
            r#"{"tools":[{"invocationName":"x","other":1}]}"#,
            r#"{"tools":[{"invocationName":"x","intent":1}]}"#,
            r#"{"tools":"git","context":[]}"#,
            r#"{"tools":"git","context":{"platform":1}}"#,
            r#"{"tools":"git","context":{"symbols":{"a":1}}}"#,
            r#"{"tools":"git","context":{"allowWeakIdentityFallback":"yes"}}"#,
            r#"{"tools":"git","context":{"unknown":true}}"#,
        ] {
            let error = parse_resolve_request(text).unwrap_err();
            assert_eq!(
                error.reason(),
                ErrorReason::InvalidContext,
                "{text}: {error}"
            );
        }
    }
}
