//! Profile and model lists for the composer's pickers, from either connection mode, and the
//! per-Run model overrides. Pure functions only: main.rs and acp.rs do the I/O.

use crate::gateway::Error;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Hermes' own rule: lowercase letters, digits, `-`, `_`, starting with a letter or digit, at most
/// 64. It's also what makes a name safe as a URL segment and inside the remote shell command.
pub fn valid_profile(name: &str) -> Result<String, Error> {
    let ok = name.len() <= 64
        && name.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    match ok {
        true => Ok(name.to_owned()),
        false => Err(Error::Invalid(format!("{name:?} is not a Hermes profile name"))),
    }
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Profiles {
    pub names: Vec<String>,
    /// The Gateway's default Profile (`hermes profile use`), if it's one of `names`.
    pub active: Option<String>,
}

/// `hermes profile list` prints a table: rows follow the `───` rule and end at a blank line, and
/// the sticky default is marked `◆`.
pub fn parse_profile_list(text: &str) -> Profiles {
    let mut names = Vec::new();
    let mut active = None;
    for line in text.lines().skip_while(|l| !l.contains('─')).skip(1).take_while(|l| !l.trim().is_empty()) {
        let line = line.trim_start();
        let Some(Ok(name)) = line.trim_start_matches('◆').split_whitespace().next().map(valid_profile) else { continue };
        if line.starts_with('◆') {
            active = Some(name.clone());
        }
        names.push(name);
    }
    Profiles { names, active }
}

/// Dashboard `GET /api/profiles` plus `GET /api/profiles/active`.
pub fn from_dashboard(list: &Value, active: &Value) -> Profiles {
    let names: Vec<String> = list["profiles"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| valid_profile(p["name"].as_str()?).ok())
        .collect();
    let active = active["active"].as_str().filter(|a| names.iter().any(|n| n == a)).map(str::to_owned);
    Profiles { names, active }
}

/// `/p/<name>/…` addresses a named Profile on a multiplexing gateway; `default` is the bare path.
pub fn api_segments<'a>(profile: Option<&'a str>, path: &[&'a str]) -> Vec<&'a str> {
    let prefix = profile.filter(|p| *p != "default").map(|p| ["p", p]);
    prefix.iter().flatten().copied().chain(path.iter().copied()).collect()
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct ModelChoice {
    pub provider: String,
    pub model: String,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct ModelGroup {
    pub provider: String,
    pub name: String,
    pub models: Vec<String>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Models {
    /// What "Profile default" currently resolves to, when Hermes says.
    pub default: Option<ModelChoice>,
    pub groups: Vec<ModelGroup>,
}

/// `GET /api/model/options` lists every provider Hermes knows about; only set-up ones with
/// models are offered.
pub fn from_options(payload: &Value) -> Models {
    let groups = payload["providers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| p["authenticated"] == true)
        .filter_map(|p| {
            let provider = p["slug"].as_str()?.to_owned();
            let name = p["name"].as_str().unwrap_or(&provider).to_owned();
            let models: Vec<String> = p["models"].as_array()?.iter().filter_map(|m| m.as_str().map(str::to_owned)).collect();
            (!models.is_empty()).then_some(ModelGroup { provider, name, models })
        })
        .collect();
    let default = match (payload["provider"].as_str(), payload["model"].as_str()) {
        (Some(provider), Some(model)) => Some(ModelChoice { provider: provider.into(), model: model.into() }),
        _ => None,
    };
    Models { default, groups }
}

/// ACP model ids are `provider:model` (hermes acp_adapter/model_catalog.py `encode_model_choice`).
pub fn split_acp_id(id: &str) -> Option<ModelChoice> {
    let (provider, model) = id.split_once(':')?;
    Some(ModelChoice { provider: provider.to_owned(), model: model.to_owned() })
}

pub fn acp_id(c: &ModelChoice) -> String {
    format!("{}:{}", c.provider, c.model)
}

/// The `models` field of an ACP `session/new` answer: `{ availableModels, currentModelId }`.
pub fn from_acp(state: &Value) -> Models {
    let mut groups: Vec<ModelGroup> = Vec::new();
    let choices = state["availableModels"].as_array().into_iter().flatten().filter_map(|m| split_acp_id(m["modelId"].as_str()?));
    for choice in choices {
        match groups.iter().position(|g| g.provider == choice.provider) {
            Some(i) => groups[i].models.push(choice.model),
            None => groups.push(ModelGroup { name: choice.provider.clone(), provider: choice.provider, models: vec![choice.model] }),
        }
    }
    Models { default: state["currentModelId"].as_str().and_then(split_acp_id), groups }
}

/// The ladder the API server accepts (gateway/platforms/api_server.py `_REASONING_EFFORTS`).
const REASONING: [&str; 7] = ["none", "minimal", "low", "medium", "high", "xhigh", "max"];

pub fn valid_reasoning(r: &str) -> Result<String, Error> {
    match REASONING.contains(&r) {
        true => Ok(r.to_owned()),
        false => Err(Error::Invalid(format!("{r:?} is not a reasoning level"))),
    }
}

/// The `POST /v1/runs` body. Model and Reasoning level override the Profile's defaults for this
/// Run only; leaving them out means "Profile default".
pub fn run_body(input: Value, session_id: Option<&str>, model: Option<&ModelChoice>, reasoning: Option<&str>) -> Value {
    let mut body = json!({ "input": input });
    if let Some(id) = session_id {
        body["session_id"] = json!(id);
    }
    if let Some(m) = model {
        body["provider"] = json!(m.provider);
        body["model"] = json!(m.model);
    }
    if let Some(r) = reasoning {
        body["model_options"] = json!({ "reasoning_effort": r });
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn profile_names_follow_hermes_rules() {
        for good in ["default", "orchestrator", "a", "web-2_x"] {
            assert!(valid_profile(good).is_ok(), "{good}");
        }
        for bad in ["", "-x", "_x", "Coder", "a b", "a;rm -rf ~", "../x", "p/x"] {
            assert!(valid_profile(bad).is_err(), "{bad:?}");
        }
        assert!(valid_profile(&"a".repeat(65)).is_err());
    }

    #[test]
    fn profile_table_is_parsed() {
        let out = "\n Profile          Model                        Gateway\n ───────────────    ──────────    ─────\n  default         deepseek/deepseek-v4-flash   running\n ◆orchestrator    z-ai/glm-5.3-flash           running\n  coder           —                            stopped\n\n ⚠  Profile 'talos' shares its buzz credential with default\n";
        let expected = Profiles { names: vec!["default".into(), "orchestrator".into(), "coder".into()], active: Some("orchestrator".into()) };
        assert_eq!(parse_profile_list(out), expected);
        assert_eq!(parse_profile_list(&out.replace("◆orchestrator", "◆ orchestrator")), expected);
    }

    #[test]
    fn dashboard_profiles() {
        let list = json!({ "profiles": [{ "name": "default" }, { "name": "orchestrator" }, { "name": "Bad Name" }] });
        let got = from_dashboard(&list, &json!({ "active": "orchestrator", "current": "default" }));
        assert_eq!(got, Profiles { names: vec!["default".into(), "orchestrator".into()], active: Some("orchestrator".into()) });
        // A sticky default this gateway doesn't list is not offered as the start Profile.
        assert_eq!(from_dashboard(&list, &json!({ "active": "gone" })).active, None);
    }

    #[test]
    fn named_profiles_get_a_path_prefix() {
        assert_eq!(api_segments(Some("coder"), &["v1", "runs"]), ["p", "coder", "v1", "runs"]);
        assert_eq!(api_segments(Some("default"), &["v1", "runs"]), ["v1", "runs"]);
        assert_eq!(api_segments(None, &["api", "sessions"]), ["api", "sessions"]);
    }

    #[test]
    fn only_set_up_providers_with_models_are_offered() {
        let payload = json!({ "provider": "zai", "model": "glm-5.3-flash", "providers": [
            { "slug": "zai", "name": "Z.ai", "authenticated": true, "models": ["glm-5.3-flash", "glm-5.3"] },
            { "slug": "nous", "name": "Nous Portal", "authenticated": false, "models": ["hermes-5"] },
            { "slug": "moa", "name": "Mixture", "authenticated": true, "models": [] },
        ]});
        assert_eq!(from_options(&payload), Models {
            default: Some(ModelChoice { provider: "zai".into(), model: "glm-5.3-flash".into() }),
            groups: vec![ModelGroup { provider: "zai".into(), name: "Z.ai".into(), models: vec!["glm-5.3-flash".into(), "glm-5.3".into()] }],
        });
    }

    #[test]
    fn acp_models_are_grouped_by_provider() {
        let state = json!({ "currentModelId": "openrouter:deepseek/deepseek-v4-flash", "availableModels": [
            { "modelId": "openrouter:deepseek/deepseek-v4-flash", "name": "deepseek-v4-flash" },
            { "modelId": "anthropic:claude-opus-5-5", "name": "claude-opus-5-5" },
            { "modelId": "openrouter:z-ai/glm-5.3-flash", "name": "glm-5.3-flash" },
            { "modelId": "no-provider", "name": "x" },
        ]});
        let models = from_acp(&state);
        let default = models.default.clone().unwrap();
        assert_eq!(default, ModelChoice { provider: "openrouter".into(), model: "deepseek/deepseek-v4-flash".into() });
        let shape: Vec<(&str, usize)> = models.groups.iter().map(|g| (g.provider.as_str(), g.models.len())).collect();
        assert_eq!(shape, [("openrouter", 2), ("anthropic", 1)]);
        assert_eq!(acp_id(&default), "openrouter:deepseek/deepseek-v4-flash");
    }

    #[test]
    fn run_body_carries_the_model_choice() {
        assert_eq!(run_body(json!("hi"), None, None, None), json!({ "input": "hi" }));
        let m = ModelChoice { provider: "zai".into(), model: "glm-5.3-flash".into() };
        assert_eq!(
            run_body(json!("hi"), Some("s1"), Some(&m), Some("high")),
            json!({ "input": "hi", "session_id": "s1", "provider": "zai", "model": "glm-5.3-flash",
                    "model_options": { "reasoning_effort": "high" } })
        );
    }

    #[test]
    fn reasoning_levels_are_hermes_values() {
        assert!(valid_reasoning("xhigh").is_ok());
        assert!(valid_reasoning("turbo").is_err());
        assert!(valid_reasoning("").is_err());
    }
}
