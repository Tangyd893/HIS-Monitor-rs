//! monitor-probe — 业务链路探活代理
//!
//! 独立二进制，模拟真实用户行为，以黑盒视角验证 HIS-Go 核心业务链路端到端健康状态。
//! 支持按场景优先级（Critical/Standard）以不同间隔执行探活。

pub mod engine;
pub mod scenario;

use engine::ProbeEngine;
use scenario::{ProbeScenario, ScenarioPriority};
use std::time::Duration;
use tracing::{info, warn};

/// 启动探活代理
pub async fn run(base_url: &str, interval_secs: u64) -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .init();

    let engine = ProbeEngine::new(base_url.to_string());
    let scenarios = ProbeScenario::all_scenarios(base_url);
    let critical_scenarios: Vec<ProbeScenario> = scenarios
        .iter()
        .filter(|s| s.priority == ScenarioPriority::Critical)
        .cloned()
        .collect();
    let standard_scenarios: Vec<ProbeScenario> = scenarios
        .iter()
        .filter(|s| s.priority == ScenarioPriority::Standard)
        .cloned()
        .collect();

    info!(
        base_url,
        critical = critical_scenarios.len(),
        standard = standard_scenarios.len(),
        "monitor-probe started"
    );

    let mut critical_tick = tokio::time::interval(Duration::from_secs(10));
    let mut standard_tick = tokio::time::interval(Duration::from_secs(interval_secs));

    loop {
        tokio::select! {
            _ = critical_tick.tick() => {
                for scenario in &critical_scenarios {
                    let (_results, _metrics) = engine.execute_scenario(scenario).await;
                    let failed: Vec<_> = _results.iter().filter(|r| !r.success).collect();
                    if !failed.is_empty() {
                        warn!(
                            scenario = scenario.name,
                            failed_steps = failed.len(),
                            "critical scenario has failures"
                        );
                    }
                }
            }

            _ = standard_tick.tick() => {
                for scenario in &standard_scenarios {
                    let (_results, _metrics) = engine.execute_scenario(scenario).await;
                    let failed: Vec<_> = _results.iter().filter(|r| !r.success).collect();
                    if !failed.is_empty() {
                        warn!(
                            scenario = scenario.name,
                            failed_steps = failed.len(),
                            "standard scenario has failures"
                        );
                    }
                }
            }
        }
    }
}
