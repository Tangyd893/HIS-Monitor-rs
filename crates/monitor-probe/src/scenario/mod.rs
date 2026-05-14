//! 探活场景定义
//!
//! 定义 HIS-Go 核心业务链路的黑盒探活场景。
//! 每个场景由多个步骤组成，步骤间通过变量传递上下文。

use super::engine::ProbeStep;
use std::collections::HashMap;

/// 探活场景
#[derive(Debug, Clone)]
pub struct ProbeScenario {
    /// 场景名称
    pub name: String,
    /// 场景级别（critical/standard）
    pub priority: ScenarioPriority,
    /// 步骤列表（顺序执行）
    pub steps: Vec<ProbeStep>,
    /// 初始变量
    pub init_vars: HashMap<String, String>,
    /// 出错是否继续
    pub continue_on_error: bool,
}

/// 场景优先级
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScenarioPriority {
    /// 关键业务链路（10s 间隔）
    Critical,
    /// 标准业务链路（30s 间隔）
    Standard,
}

impl ProbeScenario {
    /// 创建挂号链路场景
    pub fn registration_chain(gateway_url: &str) -> Self {
        Self {
            name: "registration_chain".into(),
            priority: ScenarioPriority::Critical,
            continue_on_error: false,
            init_vars: HashMap::new(),
            steps: vec![
                ProbeStep {
                    name: "gateway_health".into(),
                    method: "GET".into(),
                    url_template: format!("{}/health", gateway_url),
                    body: None,
                    expect_status: 200,
                    extract: vec![],
                },
                ProbeStep {
                    name: "registration_login".into(),
                    method: "POST".into(),
                    url_template: format!("{}/api/v1/auth/login", gateway_url),
                    body: Some(r#"{"username":"probe","password":"probe123"}"#.into()),
                    expect_status: 200,
                    extract: vec![("data.token".into(), "token".into())],
                },
                ProbeStep {
                    name: "get_departments".into(),
                    method: "GET".into(),
                    url_template: format!(
                        "{}/api/v1/departments?type=registration",
                        gateway_url
                    ),
                    body: None,
                    expect_status: 200,
                    extract: vec![("data[0].id".into(), "dept_id".into())],
                },
                ProbeStep {
                    name: "create_registration".into(),
                    method: "POST".into(),
                    url_template: format!("{}/api/v1/registrations", gateway_url),
                    body: Some(
                        r#"{"department_id":"{{dept_id}}","patient_name":"探活测试"}"#.into(),
                    ),
                    expect_status: 201,
                    extract: vec![("data.id".into(), "reg_id".into())],
                },
            ],
        }
    }

    /// 创建处方开药链路场景
    pub fn prescription_chain(gateway_url: &str) -> Self {
        Self {
            name: "prescription_chain".into(),
            priority: ScenarioPriority::Critical,
            continue_on_error: false,
            init_vars: HashMap::new(),
            steps: vec![
                ProbeStep {
                    name: "prescription_login".into(),
                    method: "POST".into(),
                    url_template: format!("{}/api/v1/auth/login", gateway_url),
                    body: Some(
                        r#"{"username":"doctor_probe","password":"probe123"}"#.into(),
                    ),
                    expect_status: 200,
                    extract: vec![("data.token".into(), "token".into())],
                },
                ProbeStep {
                    name: "get_drugs".into(),
                    method: "GET".into(),
                    url_template: format!("{}/api/v1/drugs?page=1&size=5", gateway_url),
                    body: None,
                    expect_status: 200,
                    extract: vec![("data[0].id".into(), "drug_id".into())],
                },
                ProbeStep {
                    name: "create_prescription".into(),
                    method: "POST".into(),
                    url_template: format!("{}/api/v1/prescriptions", gateway_url),
                    body: Some(
                        r#"{"patient_name":"测试患者","drugs":[{"drug_id":"{{drug_id}}","quantity":1}]}"#.into(),
                    ),
                    expect_status: 201,
                    extract: vec![("data.id".into(), "prescription_id".into())],
                },
            ],
        }
    }

    /// 创建收费链路场景
    pub fn billing_chain(gateway_url: &str) -> Self {
        Self {
            name: "billing_chain".into(),
            priority: ScenarioPriority::Standard,
            continue_on_error: false,
            init_vars: HashMap::new(),
            steps: vec![
                ProbeStep {
                    name: "billing_login".into(),
                    method: "POST".into(),
                    url_template: format!("{}/api/v1/auth/login", gateway_url),
                    body: Some(
                        r#"{"username":"cashier_probe","password":"probe123"}"#.into(),
                    ),
                    expect_status: 200,
                    extract: vec![("data.token".into(), "token".into())],
                },
                ProbeStep {
                    name: "create_bill".into(),
                    method: "POST".into(),
                    url_template: format!("{}/api/v1/bills", gateway_url),
                    body: Some(
                        r#"{"patient_name":"测试患者","amount":150.00,"pay_method":"wechat"}"#.into(),
                    ),
                    expect_status: 201,
                    extract: vec![("data.id".into(), "bill_id".into())],
                },
                ProbeStep {
                    name: "query_bill".into(),
                    method: "GET".into(),
                    url_template: format!("{}/api/v1/bills/{{bill_id}}", gateway_url),
                    body: None,
                    expect_status: 200,
                    extract: vec![],
                },
            ],
        }
    }

    /// 创建发药链路场景
    pub fn pharmacy_chain(gateway_url: &str) -> Self {
        Self {
            name: "pharmacy_chain".into(),
            priority: ScenarioPriority::Critical,
            continue_on_error: false,
            init_vars: HashMap::new(),
            steps: vec![
                ProbeStep {
                    name: "pharmacy_login".into(),
                    method: "POST".into(),
                    url_template: format!("{}/api/v1/auth/login", gateway_url),
                    body: Some(
                        r#"{"username":"pharmacy_probe","password":"probe123"}"#.into(),
                    ),
                    expect_status: 200,
                    extract: vec![("data.token".into(), "token".into())],
                },
                ProbeStep {
                    name: "get_pending_dispense".into(),
                    method: "GET".into(),
                    url_template: format!("{}/api/v1/dispense/pending", gateway_url),
                    body: None,
                    expect_status: 200,
                    extract: vec![],
                },
            ],
        }
    }

    /// 创建住院链路场景
    pub fn inpatient_chain(gateway_url: &str) -> Self {
        Self {
            name: "inpatient_chain".into(),
            priority: ScenarioPriority::Standard,
            continue_on_error: true,
            init_vars: HashMap::new(),
            steps: vec![
                ProbeStep {
                    name: "inpatient_login".into(),
                    method: "POST".into(),
                    url_template: format!("{}/api/v1/auth/login", gateway_url),
                    body: Some(
                        r#"{"username":"nurse_probe","password":"probe123"}"#.into(),
                    ),
                    expect_status: 200,
                    extract: vec![("data.token".into(), "token".into())],
                },
                ProbeStep {
                    name: "get_wards".into(),
                    method: "GET".into(),
                    url_template: format!("{}/api/v1/wards", gateway_url),
                    body: None,
                    expect_status: 200,
                    extract: vec![],
                },
                ProbeStep {
                    name: "get_beds".into(),
                    method: "GET".into(),
                    url_template: format!("{}/api/v1/beds?status=available", gateway_url),
                    body: None,
                    expect_status: 200,
                    extract: vec![],
                },
            ],
        }
    }

    /// 获取所有默认场景
    pub fn all_scenarios(gateway_url: &str) -> Vec<ProbeScenario> {
        vec![
            Self::registration_chain(gateway_url),
            Self::prescription_chain(gateway_url),
            Self::billing_chain(gateway_url),
            Self::pharmacy_chain(gateway_url),
            Self::inpatient_chain(gateway_url),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registration_chain_structure() {
        let chain = ProbeScenario::registration_chain("http://localhost:8080");
        assert_eq!(chain.name, "registration_chain");
        assert_eq!(chain.priority, ScenarioPriority::Critical);
        assert_eq!(chain.steps.len(), 4);
        assert_eq!(chain.steps[0].name, "gateway_health");
        assert_eq!(chain.steps[3].name, "create_registration");
    }

    #[test]
    fn test_prescription_chain_structure() {
        let chain = ProbeScenario::prescription_chain("http://localhost:8080");
        assert_eq!(chain.steps.len(), 3);
        assert_eq!(chain.steps[2].expect_status, 201);
    }

    #[test]
    fn test_all_scenarios_count() {
        let scenarios = ProbeScenario::all_scenarios("http://localhost:8080");
        assert_eq!(scenarios.len(), 5);
    }
}
