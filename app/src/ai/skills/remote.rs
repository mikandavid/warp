use ::ai::project_context::model::{ProjectContextModel, ProjectRule};
use ai::skills::{parse_skill_content_at_location, ParsedSkill, SkillProvider, SkillScope};
use remote_server::manager::{RemoteServerManager, RemoteServerManagerEvent};
use remote_server::proto::{
    BundledSkillProto, GlobalRuleProto, GlobalRulesSnapshot, HomeSkillProto, HomeSkillsSnapshot,
};
use warp_core::features::FeatureFlag;
use warp_core::safe_warn;
use warp_util::host_id::HostId;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warp_util::remote_path::RemotePath;
use warp_util::standardized_path::StandardizedPath;
use warpui::{AppContext, ModelContext, SingletonEntity};

use super::bundled::{BundledSkill, BundledSkillActivation};
use super::SkillManager;
use crate::ai::mcp::McpIntegration;

pub(crate) fn wire_remote_home_context(ctx: &mut AppContext) {
    SkillManager::handle(ctx).update(ctx, |manager, ctx| {
        manager.subscribe_to_remote_home_context(ctx);
    });
}

impl SkillManager {
    fn subscribe_to_remote_home_context(&mut self, ctx: &mut ModelContext<Self>) {
        let remote_server_manager = RemoteServerManager::handle(ctx);
        ctx.subscribe_to_model(&remote_server_manager, |me, event, ctx| match event {
            RemoteServerManagerEvent::BundledSkillsSnapshot { host_id, skills } => {
                if !FeatureFlag::BundledSkills.is_enabled() {
                    return;
                }
                // A fresh snapshot replaces any previous catalog for the
                // host (e.g. after a reconnect following a daemon upgrade).
                me.set_remote_bundled_skill(
                    host_id.clone(),
                    bundled_skill_from_protos(host_id, skills),
                );
            }
            RemoteServerManagerEvent::HomeSkillsSnapshot { host_id, snapshot } => {
                if let Some((home_dir, skills)) = home_skills_from_snapshot(host_id, snapshot) {
                    me.set_remote_home_skills(host_id.clone(), home_dir, skills);
                }
            }
            RemoteServerManagerEvent::GlobalRulesSnapshot { host_id, snapshot } => {
                if let Some(rules) = global_rules_from_snapshot(host_id, snapshot) {
                    ProjectContextModel::handle(ctx).update(ctx, |model, _| {
                        model.set_remote_global_rules(host_id.clone(), rules);
                    });
                }
            }
            RemoteServerManagerEvent::HostDisconnected { host_id } => {
                me.remove_remote_bundled_skill(host_id);
                me.remove_remote_home_skills(host_id);
                ProjectContextModel::handle(ctx).update(ctx, |model, _| {
                    model.remove_remote_global_rules(host_id);
                });
            }
            RemoteServerManagerEvent::SessionConnecting { .. }
            | RemoteServerManagerEvent::SessionConnected { .. }
            | RemoteServerManagerEvent::SessionConnectionFailed { .. }
            | RemoteServerManagerEvent::SessionDisconnected { .. }
            | RemoteServerManagerEvent::SessionReconnected { .. }
            | RemoteServerManagerEvent::SessionDeregistered { .. }
            | RemoteServerManagerEvent::HostConnected { .. }
            | RemoteServerManagerEvent::NavigatedToDirectory { .. }
            | RemoteServerManagerEvent::RepoMetadataSnapshot { .. }
            | RemoteServerManagerEvent::RepoMetadataUpdated { .. }
            | RemoteServerManagerEvent::RepoMetadataDirectoryLoaded { .. }
            | RemoteServerManagerEvent::CodebaseIndexStatusesSnapshot { .. }
            | RemoteServerManagerEvent::CodebaseIndexStatusUpdated { .. }
            | RemoteServerManagerEvent::BufferUpdated { .. }
            | RemoteServerManagerEvent::BufferConflictDetected { .. }
            | RemoteServerManagerEvent::DiffStateSnapshotReceived { .. }
            | RemoteServerManagerEvent::DiffStateMetadataUpdateReceived { .. }
            | RemoteServerManagerEvent::DiffStateFileDeltaReceived { .. }
            | RemoteServerManagerEvent::GetBranchesResponse { .. }
            | RemoteServerManagerEvent::CommitChainResponse { .. }
            | RemoteServerManagerEvent::GitPushResponse { .. }
            | RemoteServerManagerEvent::CreatePrResponse { .. }
            | RemoteServerManagerEvent::GenerateCommitMessageResponse { .. }
            | RemoteServerManagerEvent::GetCommittedBranchFilesResponse { .. }
            | RemoteServerManagerEvent::GitStatusPushReceived { .. }
            | RemoteServerManagerEvent::GitHubPrInfoPushReceived { .. }
            | RemoteServerManagerEvent::GitHubRepositoryInfoPushReceived { .. }
            | RemoteServerManagerEvent::SetupStateChanged { .. }
            | RemoteServerManagerEvent::BinaryCheckComplete { .. }
            | RemoteServerManagerEvent::BinaryInstallComplete { .. }
            | RemoteServerManagerEvent::ClientRequestFailed { .. }
            | RemoteServerManagerEvent::CodebaseIndexMutationFailed { .. }
            | RemoteServerManagerEvent::ServerMessageDecodingError { .. } => {}
        });
    }
}

fn skill_provider_wire_id(provider: SkillProvider) -> &'static str {
    match provider {
        SkillProvider::Warp => "warp",
        SkillProvider::Agents => "agents",
        SkillProvider::Claude => "claude",
        SkillProvider::Codex => "codex",
        SkillProvider::Cursor => "cursor",
        SkillProvider::Gemini => "gemini",
        SkillProvider::Copilot => "copilot",
        SkillProvider::Droid => "droid",
        SkillProvider::Github => "github",
        SkillProvider::OpenCode => "opencode",
    }
}

fn skill_provider_from_wire_id(wire_id: &str) -> Option<SkillProvider> {
    match wire_id {
        "warp" => Some(SkillProvider::Warp),
        "agents" => Some(SkillProvider::Agents),
        "claude" => Some(SkillProvider::Claude),
        "codex" => Some(SkillProvider::Codex),
        "cursor" => Some(SkillProvider::Cursor),
        "gemini" => Some(SkillProvider::Gemini),
        "copilot" => Some(SkillProvider::Copilot),
        "droid" => Some(SkillProvider::Droid),
        "github" => Some(SkillProvider::Github),
        "opencode" => Some(SkillProvider::OpenCode),
        _ => None,
    }
}

fn remote_home_path(host_id: &HostId, home_dir: &str) -> Option<LocalOrRemotePath> {
    StandardizedPath::try_new(home_dir)
        .ok()
        .map(|path| LocalOrRemotePath::Remote(RemotePath::new(host_id.clone(), path)))
}

fn remote_path_within_home(
    host_id: &HostId,
    path: &str,
    home_dir: &LocalOrRemotePath,
) -> Option<LocalOrRemotePath> {
    let path = StandardizedPath::try_new(path).ok()?;
    let path = LocalOrRemotePath::Remote(RemotePath::new(host_id.clone(), path));
    path.starts_with(home_dir).then_some(path)
}

fn home_skills_from_snapshot(
    host_id: &HostId,
    snapshot: &HomeSkillsSnapshot,
) -> Option<(LocalOrRemotePath, Vec<ParsedSkill>)> {
    let home_dir = remote_home_path(host_id, &snapshot.home_dir)?;
    let skills = snapshot
        .skills
        .iter()
        .filter_map(|proto| {
            let provider = skill_provider_from_wire_id(&proto.provider)?;
            let path = remote_path_within_home(host_id, &proto.path, &home_dir)?;
            parse_skill_content_at_location(path, &proto.content, provider, SkillScope::Home)
                .map_err(|error| {
                    safe_warn!(
                        safe: ("Skipping remote home skill that failed to parse"),
                        full: ("Skipping remote home skill that failed to parse: {error:#}")
                    );
                })
                .ok()
        })
        .collect();
    Some((home_dir, skills))
}

fn global_rules_from_snapshot(
    host_id: &HostId,
    snapshot: &GlobalRulesSnapshot,
) -> Option<Vec<ProjectRule>> {
    let home_dir = remote_home_path(host_id, &snapshot.home_dir)?;
    Some(
        snapshot
            .rules
            .iter()
            .filter_map(|rule| {
                Some(ProjectRule {
                    path: remote_path_within_home(host_id, &rule.path, &home_dir)?,
                    content: rule.content.clone(),
                })
            })
            .collect(),
    )
}

pub(crate) fn home_skills_snapshot(ctx: &AppContext) -> HomeSkillsSnapshot {
    let home_dir = dirs::home_dir()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut skills = SkillManager::as_ref(ctx)
        .local_home_skills()
        .into_iter()
        .map(|skill| HomeSkillProto {
            path: skill.path.display_path(),
            content: skill.content,
            provider: skill_provider_wire_id(skill.provider).to_owned(),
        })
        .collect::<Vec<_>>();
    skills.sort_by(|a, b| a.path.cmp(&b.path));
    HomeSkillsSnapshot { home_dir, skills }
}

pub(crate) fn global_rules_snapshot(ctx: &AppContext) -> GlobalRulesSnapshot {
    let home_dir = dirs::home_dir()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut rules = ProjectContextModel::as_ref(ctx)
        .local_global_rules()
        .into_iter()
        .map(|rule| GlobalRuleProto {
            path: rule.path.display_path(),
            content: rule.content,
        })
        .collect::<Vec<_>>();
    rules.sort_by(|a, b| a.path.cmp(&b.path));
    GlobalRulesSnapshot { home_dir, rules }
}

/// Stable wire identifier for an MCP integration in [`BundledSkillProto`].
fn mcp_integration_wire_id(integration: McpIntegration) -> &'static str {
    match integration {
        McpIntegration::Figma => "figma",
    }
}

fn mcp_integration_from_wire_id(wire_id: &str) -> Option<McpIntegration> {
    match wire_id {
        "figma" => Some(McpIntegration::Figma),
        _ => None,
    }
}

/// Converts a daemon-pushed snapshot into a catalog whose skill paths are
/// remote paths on `host_id`.
fn bundled_skill_from_protos(host_id: &HostId, skills: &[BundledSkillProto]) -> BundledSkill {
    let definitions = skills.iter().filter_map(|proto| {
        let path = match StandardizedPath::try_new(&proto.path) {
            Ok(path) => LocalOrRemotePath::Remote(RemotePath::new(host_id.clone(), path)),
            Err(_) => {
                safe_warn!(
                    safe: ("Skipping bundled skill with an invalid remote path"),
                    full: ("Skipping bundled skill {} with an invalid remote path: {}", proto.id, proto.path)
                );
                return None;
            }
        };
        // Re-parse the daemon-rendered content so name, description, and
        // line range are derived exactly as they are for local skills.
        let skill = match parse_skill_content_at_location(
            path,
            &proto.content,
            SkillProvider::Warp,
            SkillScope::Bundled,
        ) {
            Ok(skill) => skill,
            Err(err) => {
                safe_warn!(
                    safe: ("Skipping bundled skill that failed to parse"),
                    full: ("Skipping bundled skill {} that failed to parse: {err:#}", proto.id)
                );
                return None;
            }
        };
        let activation = match proto.requires_mcp.as_deref() {
            None => BundledSkillActivation::Always,
            Some(wire_id) => match mcp_integration_from_wire_id(wire_id) {
                Some(integration) => BundledSkillActivation::RequiresMcp(integration),
                None => {
                    // Unknown integration (e.g. a newer daemon): the client
                    // cannot evaluate the condition, so skip the skill.
                    safe_warn!(
                        safe: ("Skipping bundled skill with an unknown MCP integration"),
                        full: ("Skipping bundled skill {} with an unknown MCP integration: {wire_id}", proto.id)
                    );
                    return None;
                }
            },
        };
        Some((proto.id.clone(), skill, activation))
    });
    BundledSkill::from_definitions(definitions)
}

/// Serializes a daemon-side catalog for the `BundledSkillsSnapshot` push.
///
/// `RequiresFile` activations are evaluated here — the daemon owns the
/// files — so the client only ever receives `Always` or `RequiresMcp`
/// conditions. The result is sorted by skill ID so pushes are
/// deterministic across daemon restarts.
pub(crate) fn bundled_skills_snapshot_protos(catalog: &BundledSkill) -> Vec<BundledSkillProto> {
    let mut protos: Vec<BundledSkillProto> = catalog
        .iter_definitions()
        .filter_map(|(id, skill, activation)| {
            let requires_mcp = match activation {
                BundledSkillActivation::Always => None,
                BundledSkillActivation::RequiresMcp(integration) => {
                    Some(mcp_integration_wire_id(*integration).to_owned())
                }
                BundledSkillActivation::RequiresFeature(feature) => {
                    if !feature.is_enabled() {
                        return None;
                    }
                    None
                }
                BundledSkillActivation::RequiresFile(path) => {
                    if !path.exists() {
                        return None;
                    }
                    None
                }
            };
            Some(BundledSkillProto {
                id: id.to_owned(),
                name: skill.name.clone(),
                description: skill.description.clone(),
                path: skill.path.display_path(),
                content: skill.content.clone(),
                requires_mcp,
            })
        })
        .collect();
    protos.sort_by(|a, b| a.id.cmp(&b.id));
    protos
}

#[cfg(test)]
#[path = "remote_tests.rs"]
mod tests;
