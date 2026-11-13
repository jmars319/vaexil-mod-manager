//! A mod's options as SMF defines them (`docs/Manifest.md` and `getConfig` in
//! the Mod Manager's `lib/utils.ts`, SMF 2.33.45). Only checkbox and select
//! options can be chosen; conditional ones are never shown, as in SMF.
//!
//! - A checkbox is stored as its `name`; a select as `group:name`, and only
//!   one per group is used.
//! - An option whose `requirements` are not all in the load order is not
//!   used: SMF drops it from `modOptions` on its next read.

use serde_json::Value;

use crate::framework::ModOption;

struct Choice<'a> {
    value: String,
    name: &'a str,
    group: Option<&'a str>,
    tooltip: Option<&'a str>,
    enabled_by_default: bool,
    requirements: Vec<&'a str>,
}

/// What SMF stores for this option when it is chosen: `name` for a checkbox,
/// `group:name` for a select. `None` for options that can't be chosen.
pub fn value_of(option: &Value) -> Option<String> {
    let name = option.get("name")?.as_str()?;
    match option.get("type")?.as_str()? {
        "checkbox" => Some(name.to_string()),
        "select" => Some(format!("{}:{name}", option.get("group")?.as_str()?)),
        _ => None,
    }
}

/// The checkbox and select options in a manifest's `options`, in order.
fn choices(options: &[Value]) -> Vec<Choice<'_>> {
    options
        .iter()
        .filter_map(|o| {
            let name = o.get("name")?.as_str()?;
            let group = match o.get("type")?.as_str()? {
                "checkbox" => None,
                "select" => Some(o.get("group")?.as_str()?),
                _ => return None,
            };
            Some(Choice {
                value: group.map_or_else(|| name.to_string(), |g| format!("{g}:{name}")),
                name,
                group,
                tooltip: o.get("tooltip").and_then(Value::as_str).filter(|t| !t.is_empty()),
                enabled_by_default: o.get("enabledByDefault").and_then(Value::as_bool).unwrap_or(false),
                requirements: o
                    .get("requirements")
                    .and_then(Value::as_array)
                    .map(|r| r.iter().filter_map(Value::as_str).collect())
                    .unwrap_or_default(),
            })
        })
        .collect()
}

fn missing<'a>(choice: &Choice<'a>, load_order: &[String]) -> Vec<&'a str> {
    choice.requirements.iter().copied().filter(|r| !load_order.iter().any(|id| id == r)).collect()
}

/// What SMF selects the first time a mod is turned on: options marked
/// `enabledByDefault` whose requirements are in the load order.
pub fn default_values(options: &[Value], load_order: &[String]) -> Vec<String> {
    choices(options)
        .into_iter()
        .filter(|c| c.enabled_by_default && missing(c, load_order).is_empty())
        .map(|c| c.value)
        .collect()
}

/// The options to show for a mod. `stored` is its `modOptions` entry, if it
/// has one; without one, the defaults are shown, since turning the mod on
/// selects them.
pub fn describe(options: &[Value], stored: Option<&[String]>, load_order: &[String]) -> Vec<ModOption> {
    let defaults;
    let chosen: &[String] = match stored {
        Some(s) => s,
        None => {
            defaults = default_values(options, load_order);
            &defaults
        }
    };
    choices(options)
        .into_iter()
        .map(|c| ModOption {
            chosen: chosen.contains(&c.value),
            needs: missing(&c, load_order).into_iter().map(str::to_string).collect(),
            label: c.name.to_string(),
            group: c.group.map(str::to_string),
            tooltip: c.tooltip.map(str::to_string),
            value: c.value,
        })
        .collect()
}

/// Checks a choice against the manifest and puts it in manifest order, ready
/// to store. Every select group gets one choice: the one picked, or failing
/// that its default, or failing that its first option, counting only options
/// whose required mods are on. A group with none of those gets no choice.
pub fn validate(options: &[Value], picked: &[String], load_order: &[String]) -> Result<Vec<String>, String> {
    let all = choices(options);
    for p in picked {
        let Some(c) = all.iter().find(|c| c.value == *p) else {
            return Err(format!("\"{p}\" is not one of this mod's options."));
        };
        let needs = missing(c, load_order);
        if !needs.is_empty() {
            return Err(format!("\"{}\" needs {} turned on first.", c.name, needs.join(", ")));
        }
    }
    let mut out = Vec::new();
    let mut groups_done: Vec<&str> = Vec::new();
    for c in &all {
        match c.group {
            None => {
                if picked.contains(&c.value) {
                    out.push(c.value.clone());
                }
            }
            Some(g) if !groups_done.contains(&g) => {
                groups_done.push(g);
                let in_group: Vec<&Choice> = all.iter().filter(|o| o.group == Some(g)).collect();
                let chosen: Vec<&&Choice> = in_group.iter().filter(|o| picked.contains(&o.value)).collect();
                if chosen.len() > 1 {
                    return Err(format!("Only one choice can be picked for \"{g}\"."));
                }
                let usable: Vec<&&Choice> = in_group.iter().filter(|o| missing(o, load_order).is_empty()).collect();
                let pick = chosen
                    .first()
                    .map(|o| **o)
                    .or_else(|| usable.iter().find(|o| o.enabled_by_default).map(|o| **o))
                    .or_else(|| usable.first().map(|o| **o));
                if let Some(o) = pick {
                    out.push(o.value.clone());
                }
            }
            Some(_) => {}
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn manifest() -> Value {
        json!([
            { "name": "Darker", "type": "checkbox", "enabledByDefault": true, "tooltip": "Darker suits" },
            { "name": "Brighter", "type": "checkbox" },
            { "name": "Bare hands", "group": "Hands", "type": "select", "enabledByDefault": true },
            { "name": "Gloves", "group": "Hands", "type": "select" },
            { "name": "With A", "type": "checkbox", "enabledByDefault": true, "requirements": ["Mod.A"] },
            { "name": "Epic", "type": "conditional", "condition": "true" },
            { "name": "Odd" },
        ])
    }

    fn opts() -> Vec<Value> {
        manifest().as_array().unwrap().clone()
    }

    #[test]
    fn defaults_skip_options_whose_requirements_are_off() {
        assert_eq!(default_values(&opts(), &[]), ["Darker", "Hands:Bare hands"]);
        assert_eq!(default_values(&opts(), &["Mod.A".into()]), ["Darker", "Hands:Bare hands", "With A"]);
    }

    #[test]
    fn describes_checkboxes_and_selects_only() {
        let stored = vec!["Brighter".to_string(), "Hands:Gloves".to_string()];
        let shown = describe(&opts(), Some(&stored), &[]);
        let labels: Vec<(&str, bool)> = shown.iter().map(|o| (o.label.as_str(), o.chosen)).collect();
        assert_eq!(
            labels,
            [("Darker", false), ("Brighter", true), ("Bare hands", false), ("Gloves", true), ("With A", false)]
        );
        assert_eq!(shown[0].tooltip.as_deref(), Some("Darker suits"));
        assert_eq!(shown[3].group.as_deref(), Some("Hands"));
        assert_eq!(shown[4].needs, ["Mod.A"]);

        // No entry yet: the defaults are what turning it on would pick.
        let chosen: Vec<String> =
            describe(&opts(), None, &[]).into_iter().filter(|o| o.chosen).map(|o| o.value).collect();
        assert_eq!(chosen, ["Darker", "Hands:Bare hands"]);
    }

    #[test]
    fn validate_orders_and_fills_every_group() {
        let picked = vec!["Brighter".to_string(), "Darker".to_string()];
        assert_eq!(validate(&opts(), &picked, &[]).unwrap(), ["Darker", "Brighter", "Hands:Bare hands"]);
        let picked = vec!["Hands:Gloves".to_string()];
        assert_eq!(validate(&opts(), &picked, &[]).unwrap(), ["Hands:Gloves"]);
    }

    #[test]
    fn a_group_whose_options_all_need_mods_gets_no_choice() {
        let opts = json!([
            { "name": "Calm", "group": "Mood", "type": "select", "requirements": ["Mod.A"] },
            { "name": "Angry", "group": "Mood", "type": "select", "enabledByDefault": true, "requirements": ["Mod.A"] },
            { "name": "Loud", "group": "Volume", "type": "select", "requirements": ["Mod.A"] },
            { "name": "Quiet", "group": "Volume", "type": "select" },
            { "name": "Darker", "type": "checkbox" },
        ]);
        let opts = opts.as_array().unwrap();
        assert_eq!(validate(opts, &["Darker".into()], &[]).unwrap(), ["Volume:Quiet", "Darker"]);
        assert_eq!(validate(opts, &[], &["Mod.A".into()]).unwrap(), ["Mood:Angry", "Volume:Loud"]);
    }

    #[test]
    fn validate_refuses_what_smf_would_drop() {
        assert!(validate(&opts(), &["Nope".into()], &[]).unwrap_err().contains("not one of"));
        assert!(validate(&opts(), &["With A".into()], &[]).unwrap_err().contains("needs Mod.A"));
        assert!(validate(&opts(), &["With A".into()], &["Mod.A".into()]).is_ok());
        let both = vec!["Hands:Bare hands".to_string(), "Hands:Gloves".to_string()];
        assert!(validate(&opts(), &both, &[]).unwrap_err().contains("Only one"));
        assert!(validate(&opts(), &["Epic".into()], &[]).is_err());
    }
}
