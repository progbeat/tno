use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct CheckConfig {
    pub(crate) version: u32,
    pub(crate) agent: AgentConfig,
    pub(crate) expectations: Vec<Expectation>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawCheckConfig {
    pub(crate) version: u32,
    pub(crate) agent: AgentConfig,
    pub(crate) expectations: Vec<RawExpectationItem>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct AgentConfig {
    #[serde(default)]
    pub(crate) model: ModelConfig,
    #[serde(default = "default_thinking")]
    pub(crate) thinking: String,
    pub(crate) instructions: String,
    pub(crate) ignore: Vec<String>,
    pub(crate) plugins: Vec<String>,
}

#[derive(Debug, Deserialize, Clone, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelConfig {
    #[serde(default)]
    pub(crate) primary: Option<String>,
    #[serde(default)]
    pub(crate) fallbacks: Vec<String>,
}

pub(crate) fn default_thinking() -> String {
    "low".to_string()
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct Expectation {
    pub(crate) q: String,
    pub(crate) a: String,
    #[serde(default)]
    pub(crate) cooldown: Option<String>,
    #[serde(default)]
    pub(crate) thinking: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) enum RawExpectationItem {
    Explicit(RawExplicitExpectation),
    Generator(RawGeneratorExpectation),
    Include(RawIncludeExpectation),
}

#[derive(Debug, Clone)]
pub(crate) struct RawExplicitExpectation {
    pub(crate) q: String,
    pub(crate) a: String,
    pub(crate) cooldown: Option<String>,
    pub(crate) thinking: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct RawGeneratorExpectation {
    pub(crate) q_template: String,
    pub(crate) path: String,
    pub(crate) a: String,
    pub(crate) cooldown: Option<String>,
    pub(crate) thinking: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct RawIncludeExpectation {
    pub(crate) include: String,
}

#[derive(Debug, Deserialize)]
struct RawExpectationFields {
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    q_template: Option<String>,
    #[serde(default)]
    a: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    include: Option<String>,
    #[serde(default)]
    cooldown: Option<String>,
    #[serde(default)]
    thinking: Option<String>,
}

impl<'de> Deserialize<'de> for RawExpectationItem {
    fn deserialize<D>(deserializer: D) -> Result<RawExpectationItem, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let fields = RawExpectationFields::deserialize(deserializer)?;
        RawExpectationItem::from_fields(fields).map_err(serde::de::Error::custom)
    }
}

impl RawExpectationItem {
    fn from_fields(fields: RawExpectationFields) -> Result<RawExpectationItem, &'static str> {
        match (
            fields.q,
            fields.q_template,
            fields.path,
            fields.include,
            fields.a,
        ) {
            (Some(q), None, None, None, Some(a)) => {
                Ok(RawExpectationItem::Explicit(RawExplicitExpectation {
                    q,
                    a,
                    cooldown: fields.cooldown,
                    thinking: fields.thinking,
                }))
            }
            (None, Some(q_template), Some(path), None, Some(a)) => {
                Ok(RawExpectationItem::Generator(RawGeneratorExpectation {
                    q_template,
                    path,
                    a,
                    cooldown: fields.cooldown,
                    thinking: fields.thinking,
                }))
            }
            (None, None, None, Some(include), None) => {
                Ok(RawExpectationItem::Include(RawIncludeExpectation {
                    include,
                }))
            }
            (_, _, _, Some(_), Some(_)) => Err("include item must not contain a"),
            (Some(_), _, _, Some(_), _) => Err("include item must not contain q"),
            (_, Some(_), _, Some(_), _) => Err("include item must not contain q_template"),
            (_, _, Some(_), Some(_), _) => Err("include item must not contain path"),
            (Some(_), Some(_), _, _, _) => Err("must not contain both q and q_template"),
            (Some(_), None, Some(_), _, _) => {
                Err("must not contain path on an explicit expectation")
            }
            (Some(_), None, None, None, None) => Err("must contain a"),
            (None, Some(_), None, _, _) => Err("generator must contain path"),
            (None, Some(_), Some(_), None, None) => Err("must contain a"),
            (None, None, Some(_), _, _) => Err("generator must contain q_template"),
            (None, None, None, None, Some(_)) => Err("must contain q or q_template"),
            (None, None, None, None, None) => Err("must contain q, q_template, or include"),
        }
    }
}
