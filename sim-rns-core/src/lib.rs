use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Recipe {
    pub metadata: RecipeMetadata,
    pub vm: VmSetup,
    pub templates: Vec<Template>,
    pub elements: Vec<Element>,
    pub topology: Topology,
    pub startup: StartupPlan,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecipeMetadata {
    pub id: String,
    pub name: String,
    pub description: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct VmSetup {
    pub base_image: String,
    pub os_family: String,
    pub ram_mb: u32,
    pub cpu_cores: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Template {
    pub id: String,
    pub label: String,
    pub category: TemplateCategory,
    pub extends: Option<String>,
    pub description: String,
    pub runtime: RuntimeSpec,
    pub defaults: TemplateDefaults,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TemplateCategory {
    Reticulum,
    Network,
    Script,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeSpec {
    pub family: RuntimeFamily,
    pub image_features: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeFamily {
    Binary,
    Python,
    Bash,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TemplateDefaults {
    pub command: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub restart_policy: RestartPolicy,
    pub resources: ResourceLimits,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Element {
    pub id: String,
    pub template_id: String,
    pub enabled: bool,
    pub env: BTreeMap<String, String>,
    pub assets: Vec<AssetSeed>,
    pub restart_policy: Option<RestartPolicy>,
    pub resources: Option<ResourceLimits>,
    pub command_override: Option<Vec<String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AssetSeed {
    pub source: String,
    pub destination: String,
    pub mode: AssetMode,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AssetMode {
    Copy,
    Template,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RestartPolicy {
    Never,
    OnFailure,
    Always,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        Self::OnFailure
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ResourceLimits {
    pub memory_mb: u32,
    pub cpu_weight: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Topology {
    pub attachments: Vec<Attachment>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Attachment {
    pub element_id: String,
    pub network_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct StartupPlan {
    pub order: Vec<String>,
}

pub fn base_templates() -> Vec<Template> {
    vec![
        Template {
            id: "rns.rs.backbone".to_string(),
            label: "RNS Rust Backbone".to_string(),
            category: TemplateCategory::Reticulum,
            extends: None,
            description: "Rust backbone node powered by rnsd.".to_string(),
            runtime: RuntimeSpec {
                family: RuntimeFamily::Binary,
                image_features: vec!["rns-rs".to_string()],
            },
            defaults: TemplateDefaults {
                command: vec!["rnsd".to_string()],
                env: BTreeMap::new(),
                restart_policy: RestartPolicy::OnFailure,
                resources: ResourceLimits {
                    memory_mb: 256,
                    cpu_weight: 100,
                },
            },
        },
        Template {
            id: "lxmf.rs.client".to_string(),
            label: "LXMF Rust Client".to_string(),
            category: TemplateCategory::Reticulum,
            extends: None,
            description: "Rust LXMF client actor.".to_string(),
            runtime: RuntimeSpec {
                family: RuntimeFamily::Binary,
                image_features: vec!["lxmf-rs".to_string()],
            },
            defaults: TemplateDefaults {
                command: vec!["lxmfd".to_string()],
                env: BTreeMap::new(),
                restart_policy: RestartPolicy::OnFailure,
                resources: ResourceLimits {
                    memory_mb: 192,
                    cpu_weight: 100,
                },
            },
        },
        Template {
            id: "reticulum.python.backbone".to_string(),
            label: "Python Reticulum Backbone".to_string(),
            category: TemplateCategory::Reticulum,
            extends: None,
            description: "Python Reticulum backbone runtime.".to_string(),
            runtime: RuntimeSpec {
                family: RuntimeFamily::Python,
                image_features: vec!["python-reticulum".to_string()],
            },
            defaults: TemplateDefaults {
                command: vec!["python3".to_string(), "/opt/sim-rns/reticulum_backbone.py".to_string()],
                env: BTreeMap::new(),
                restart_policy: RestartPolicy::OnFailure,
                resources: ResourceLimits {
                    memory_mb: 256,
                    cpu_weight: 100,
                },
            },
        },
        Template {
            id: "lxmf.python.client".to_string(),
            label: "Python LXMF Client".to_string(),
            category: TemplateCategory::Reticulum,
            extends: None,
            description: "Python LXMF client runtime.".to_string(),
            runtime: RuntimeSpec {
                family: RuntimeFamily::Python,
                image_features: vec!["python-reticulum".to_string()],
            },
            defaults: TemplateDefaults {
                command: vec!["python3".to_string(), "/opt/sim-rns/lxmf_client.py".to_string()],
                env: BTreeMap::new(),
                restart_policy: RestartPolicy::OnFailure,
                resources: ResourceLimits {
                    memory_mb: 192,
                    cpu_weight: 100,
                },
            },
        },
        Template {
            id: "network.lan".to_string(),
            label: "LAN Segment".to_string(),
            category: TemplateCategory::Network,
            extends: None,
            description: "Shared network segment for attaching elements.".to_string(),
            runtime: RuntimeSpec {
                family: RuntimeFamily::Binary,
                image_features: vec!["iproute2".to_string()],
            },
            defaults: TemplateDefaults {
                command: vec!["/usr/bin/env".to_string(), "true".to_string()],
                env: BTreeMap::new(),
                restart_policy: RestartPolicy::Never,
                resources: ResourceLimits {
                    memory_mb: 32,
                    cpu_weight: 10,
                },
            },
        },
        Template {
            id: "script.python".to_string(),
            label: "Python Script".to_string(),
            category: TemplateCategory::Script,
            extends: None,
            description: "Generic Python script runner.".to_string(),
            runtime: RuntimeSpec {
                family: RuntimeFamily::Python,
                image_features: vec!["python3".to_string()],
            },
            defaults: TemplateDefaults {
                command: vec!["python3".to_string()],
                env: BTreeMap::new(),
                restart_policy: RestartPolicy::Never,
                resources: ResourceLimits {
                    memory_mb: 128,
                    cpu_weight: 50,
                },
            },
        },
        Template {
            id: "script.bash".to_string(),
            label: "Bash Script".to_string(),
            category: TemplateCategory::Script,
            extends: None,
            description: "Generic Bash script runner.".to_string(),
            runtime: RuntimeSpec {
                family: RuntimeFamily::Bash,
                image_features: vec!["bash".to_string()],
            },
            defaults: TemplateDefaults {
                command: vec!["bash".to_string()],
                env: BTreeMap::new(),
                restart_policy: RestartPolicy::Never,
                resources: ResourceLimits {
                    memory_mb: 64,
                    cpu_weight: 30,
                },
            },
        },
    ]
}

pub fn sample_recipe() -> Recipe {
    let templates = {
        let mut templates = base_templates();
        templates.push(Template {
            id: "custom.phone".to_string(),
            label: "Phone Persona".to_string(),
            category: TemplateCategory::Reticulum,
            extends: Some("lxmf.python.client".to_string()),
            description: "Custom phone-oriented LXMF profile.".to_string(),
            runtime: RuntimeSpec {
                family: RuntimeFamily::Python,
                image_features: vec!["python-reticulum".to_string()],
            },
            defaults: TemplateDefaults {
                command: vec![
                    "python3".to_string(),
                    "/opt/sim-rns/lxmf_phone.py".to_string(),
                ],
                env: BTreeMap::from([
                    ("SIM_PERSONA".to_string(), "phone".to_string()),
                    ("SIM_SLEEP_PROFILE".to_string(), "intermittent".to_string()),
                ]),
                restart_policy: RestartPolicy::OnFailure,
                resources: ResourceLimits {
                    memory_mb: 192,
                    cpu_weight: 80,
                },
            },
        });
        templates
    };

    Recipe {
        metadata: RecipeMetadata {
            id: "mesh-lab-01".to_string(),
            name: "Mesh Lab 01".to_string(),
            description: "Starter recipe for a VM-backed Reticulum experiment.".to_string(),
        },
        vm: VmSetup {
            base_image: "sim-rns-guest-v1".to_string(),
            os_family: "debian".to_string(),
            ram_mb: 4096,
            cpu_cores: 4,
        },
        templates,
        elements: vec![
            Element {
                id: "backbone-a".to_string(),
                template_id: "rns.rs.backbone".to_string(),
                enabled: true,
                env: BTreeMap::from([("RNS_INSTANCE".to_string(), "backbone-a".to_string())]),
                assets: vec![AssetSeed {
                    source: "assets/backbone-a/config.toml".to_string(),
                    destination: "config/config.toml".to_string(),
                    mode: AssetMode::Template,
                }],
                restart_policy: None,
                resources: None,
                command_override: None,
            },
            Element {
                id: "phone-a".to_string(),
                template_id: "custom.phone".to_string(),
                enabled: true,
                env: BTreeMap::from([("LXMF_DISPLAY_NAME".to_string(), "phone-a".to_string())]),
                assets: vec![],
                restart_policy: Some(RestartPolicy::OnFailure),
                resources: Some(ResourceLimits {
                    memory_mb: 160,
                    cpu_weight: 70,
                }),
                command_override: None,
            },
            Element {
                id: "lan-main".to_string(),
                template_id: "network.lan".to_string(),
                enabled: true,
                env: BTreeMap::new(),
                assets: vec![],
                restart_policy: Some(RestartPolicy::Never),
                resources: None,
                command_override: None,
            },
            Element {
                id: "traffic-seed".to_string(),
                template_id: "script.python".to_string(),
                enabled: false,
                env: BTreeMap::from([("SIM_TRIGGER".to_string(), "initial-burst".to_string())]),
                assets: vec![AssetSeed {
                    source: "scripts/traffic_seed.py".to_string(),
                    destination: "scripts/traffic_seed.py".to_string(),
                    mode: AssetMode::Copy,
                }],
                restart_policy: Some(RestartPolicy::Never),
                resources: Some(ResourceLimits {
                    memory_mb: 64,
                    cpu_weight: 20,
                }),
                command_override: Some(vec![
                    "python3".to_string(),
                    "scripts/traffic_seed.py".to_string(),
                ]),
            },
        ],
        topology: Topology {
            attachments: vec![
                Attachment {
                    element_id: "backbone-a".to_string(),
                    network_id: "lan-main".to_string(),
                },
                Attachment {
                    element_id: "phone-a".to_string(),
                    network_id: "lan-main".to_string(),
                },
            ],
        },
        startup: StartupPlan {
            order: vec![
                "lan-main".to_string(),
                "backbone-a".to_string(),
                "phone-a".to_string(),
            ],
        },
    }
}
