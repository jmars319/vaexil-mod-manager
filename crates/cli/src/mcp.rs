//! `vmm mcp`: a Model Context Protocol server over stdio (ADR 0003, step 3),
//! so Claude Code, Gemini CLI, Codex CLI or any MCP client can use Vaexil Mod
//! Manager as structured tools instead of controlling the screen.
//!
//! It opens no port and makes no network call: the only transport is the
//! client's own stdin and stdout. Every tool runs the same engine as the
//! window and the command line, and returns the same JSON (`vmm schema`).
//! Reading is always allowed. Changing is `plan_change` then `apply_plan`,
//! and `apply_plan` is refused unless the user has turned on "Let AI tools
//! make changes" in Settings; it also needs the user's own request, which the
//! journal keeps beside the change.

use rmcp::handler::server::common::schema_for_output;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::{ErrorData as McpError, Peer, RoleServer, ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use vmm_core::error::VmmError;
use vmm_core::schema::{
    ApplyReport, Change, DeployStatus, Diagnosis, ErrorCodeList, ErrorReport, GameList, LastDeploy, LogList, ModDetail,
    ModList, OrderReport, Plan, SCHEMA_VERSION, SetupList, StatusReport,
};
use vmm_engine::Engine;
use vmm_engine::journal::Caller;

/// What the server tells the model when it connects.
pub const INSTRUCTIONS: &str = "\
Vaexil Mod Manager manages mods for HITMAN World of Assassination through Simple Mod Framework (SMF).

Rules:
- You may read anything at any time: get_status, list_mods, get_mod, get_load_order, list_games, \
list_error_codes, get_logs, get_deploy_report, diagnose. For \"what went wrong\", start with diagnose.
- Change nothing on your own initiative. Plan or apply a change only when the user has asked for that \
change in so many words, such as \"turn off every mod\" or \"enable these three\".
- To change something: call plan_change, show the user the plan's steps, then call apply_plan with the \
planId and the user's request in their own words. A plan expires after 10 minutes and is used once.
- Changes include turning mods on or off, options, types, the load order, auto-sort, setups (use, save, delete) \
and the compatibility pass (startPass, tryBatch, markBatch, endPass). A setup or pass step is planned on its own.
- For a compatibility pass after a game update: start the pass (usually from every mod off), try small batches \
the user picks, and after the user has deployed and played, mark each batch good or bad as the user says.
- start_deploy runs only when the user asks for a deploy; follow it with get_deploy_status.
- If apply_plan fails with ai.changes_off, the user has not allowed changes from AI tools. Tell them; don't \
look for another way.
- Changes reach the game only after a deploy, which the user starts in Vaexil Mod Manager.
- Every error has a stable code, a message, a hint and whether retrying can work. Follow the hint.";

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct GameArgs {
    /// The game folder. Leave it out for the first install found (see list_games).
    #[serde(default)]
    pub game: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ModArgs {
    /// The mod's id, as list_mods gives it.
    #[serde(rename = "mod")]
    pub mod_id: String,
    #[serde(default)]
    pub game: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeployArgs {
    /// What the user asked for, in their own words. Required: a deploy
    /// rewrites the game's patch files.
    pub request: String,
    #[serde(default)]
    pub game: Option<String>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct LogArgs {
    /// How many of the newest events to return (default 50).
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PlanArgs {
    /// The changes, applied in this order. Each is checked against the state
    /// the changes before it leave.
    pub changes: Vec<Change>,
    #[serde(default)]
    pub game: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApplyArgs {
    /// The planId plan_change returned.
    pub plan_id: String,
    /// What the user asked for, in their own words. Required: AI tools change
    /// nothing on their own initiative, and the journal keeps this.
    pub request: String,
    #[serde(default)]
    pub game: Option<String>,
}

fn result<T: Serialize>(r: Result<T, VmmError>) -> Result<CallToolResult, McpError> {
    Ok(match r {
        Ok(v) => CallToolResult::structured(serde_json::to_value(v).expect("results serialize")),
        // An error the model can act on: the same ErrorReport `vmm --json`
        // prints, as text, so it never has to match the success schema.
        Err(e) => CallToolResult::error(vec![ContentBlock::text(
            serde_json::to_string_pretty(&ErrorReport::from(&e)).expect("errors serialize"),
        )]),
    })
}

fn engine_for(game: &Option<String>) -> (Engine, Option<std::path::PathBuf>) {
    engine_as(game, Caller::Mcp { client: "unknown".into() })
}

fn engine_as(game: &Option<String>, caller: Caller) -> (Engine, Option<std::path::PathBuf>) {
    (Engine::open_as(caller), game.as_ref().map(std::path::PathBuf::from))
}

fn client_name(peer: &Peer<RoleServer>) -> String {
    peer.peer_info().map(|i| i.client_info.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| "unknown".into())
}

#[derive(Clone)]
pub struct Vmm {
    tool_router: ToolRouter<Self>,
    /// A deploy started here, which outlives the call that started it.
    progress: vmm_engine::deploy::Shared,
}

impl Default for Vmm {
    fn default() -> Self {
        Self::new()
    }
}

#[tool_router(router = tool_router)]
impl Vmm {
    pub fn new() -> Self {
        Self { tool_router: Self::tool_router(), progress: Default::default() }
    }

    #[tool(
        description = "Every install of the game found. Other tools act on the first unless given `game`.",
        output_schema = schema_for_output::<GameList>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn list_games(&self) -> Result<CallToolResult, McpError> {
        result(Ok::<_, VmmError>(Engine::open().games()))
    }

    #[tool(
        description = "The game, the mod framework (SMF) and its version, how many mods are on, whether the game \
                       has the current mods (deployed), saved setups, and whether a compatibility pass is under way.",
        output_schema = schema_for_output::<StatusReport>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn get_status(&self, Parameters(a): Parameters<GameArgs>) -> Result<CallToolResult, McpError> {
        let (e, folder) = engine_for(&a.game);
        let game = match e.pick_game(folder.as_deref()) {
            Ok(g) => Some(g),
            Err(err) if folder.is_some() => return result::<StatusReport>(Err(err)),
            Err(_) => None,
        };
        result(Ok::<_, VmmError>(e.status(game.as_ref())))
    }

    #[tool(
        description = "Every mod: id, name, on or off, place in the load order, type, options (with the value to \
                       choose each), load rules, and which later mods replace its files.",
        output_schema = schema_for_output::<ModList>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn list_mods(&self, Parameters(a): Parameters<GameArgs>) -> Result<CallToolResult, McpError> {
        let (e, folder) = engine_for(&a.game);
        result(e.pick_game(folder.as_deref()).and_then(|g| e.mod_list(&g)))
    }

    #[tool(
        description = "One mod by its id.",
        output_schema = schema_for_output::<ModDetail>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn get_mod(&self, Parameters(a): Parameters<ModArgs>) -> Result<CallToolResult, McpError> {
        let (e, folder) = engine_for(&a.game);
        result(e.pick_game(folder.as_deref()).and_then(|g| e.mod_detail(&g, &a.mod_id)))
    }

    #[tool(
        description = "The load order of the mods that are on (a later mod wins), the authors' ordering rules it \
                       breaks, and an order that keeps them.",
        output_schema = schema_for_output::<OrderReport>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn get_load_order(&self, Parameters(a): Parameters<GameArgs>) -> Result<CallToolResult, McpError> {
        let (e, folder) = engine_for(&a.game);
        result(e.pick_game(folder.as_deref()).and_then(|g| e.order_report(&g)))
    }

    #[tool(
        description = "Every error code: what it means, what to do, whether retrying can work.",
        output_schema = schema_for_output::<ErrorCodeList>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn list_error_codes(&self) -> Result<CallToolResult, McpError> {
        result(Ok::<_, VmmError>(ErrorCodeList::current()))
    }

    #[tool(
        description = "The newest events, newest last: every change (with who made it: gui, cli or mcp:<app>) \
                       and every failure, with its error code.",
        output_schema = schema_for_output::<LogList>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn get_logs(&self, Parameters(a): Parameters<LogArgs>) -> Result<CallToolResult, McpError> {
        let events = engine_for(&None).0.recent_events(a.limit.unwrap_or(50));
        result(Ok::<_, VmmError>(LogList { schema_version: SCHEMA_VERSION, events }))
    }

    #[tool(
        description = "The last deploy, read from SMF's output: whether it succeeded, how long it took, its \
                       stages, its warnings grouped by kind and mod (each counted once), and its errors.",
        output_schema = schema_for_output::<LastDeploy>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn get_deploy_report(&self) -> Result<CallToolResult, McpError> {
        let report = engine_for(&None).0.deploy_report();
        result(Ok::<_, VmmError>(LastDeploy { schema_version: SCHEMA_VERSION, report }))
    }

    #[tool(
        description = "Everything at once for working out a problem: how the game was found, the status, every \
                       current problem (unreadable mods, broken load rules), the last deploy, the newest 200 \
                       events, settings and any crash records. Paths under the user's profile start with ~.",
        output_schema = schema_for_output::<Diagnosis>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn diagnose(&self, Parameters(a): Parameters<GameArgs>) -> Result<CallToolResult, McpError> {
        let (e, folder) = engine_for(&a.game);
        let game = match e.pick_game(folder.as_deref()) {
            Ok(g) => Some(g),
            Err(err) if folder.is_some() => return result::<Diagnosis>(Err(err)),
            Err(_) => None,
        };
        result(Ok::<_, VmmError>(e.diagnose(game.as_ref())))
    }

    #[tool(
        description = "Saved setups (each a set of mods on, in order, with options) and the compatibility pass \
                       under way: its baseline, each batch tried with its result, and the mods that can go into \
                       the next batch.",
        output_schema = schema_for_output::<SetupList>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn get_setups(&self, Parameters(a): Parameters<GameArgs>) -> Result<CallToolResult, McpError> {
        let (e, folder) = engine_for(&a.game);
        let game = e.pick_game(folder.as_deref()).ok();
        result(e.setup_list(game.as_ref()))
    }

    #[tool(
        description = "Starts SMF's Deploy.exe, which puts the mods into the game; it can take many minutes. Only \
                       when the user asked for a deploy; pass their request. Refused (ai.changes_off) unless the \
                       user allows changes from AI tools in Settings, and while the game or a deploy is running. \
                       Follow it with get_deploy_status.",
        output_schema = schema_for_output::<DeployStatus>(),
        annotations(read_only_hint = false, destructive_hint = true, idempotent_hint = false, open_world_hint = false)
    )]
    async fn start_deploy(
        &self,
        Parameters(a): Parameters<DeployArgs>,
        peer: Peer<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let caller = Caller::Mcp { client: client_name(&peer) };
        let (mut e, folder) = engine_as(&a.game, caller.clone());
        e.share_progress(self.progress.clone());
        result(
            e.pick_game(folder.as_deref())
                .and_then(|g| e.start_deploy_as(&g, &caller, Some(&a.request)).map(|()| e.deploy_status(Some(&g)))),
        )
    }

    #[tool(
        description = "Whether the deploy started with start_deploy is still running, its latest line, and once it \
                       has finished, its report (as get_deploy_report).",
        output_schema = schema_for_output::<DeployStatus>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = false, open_world_hint = false)
    )]
    async fn get_deploy_status(&self, Parameters(a): Parameters<GameArgs>) -> Result<CallToolResult, McpError> {
        let (mut e, folder) = engine_for(&a.game);
        e.share_progress(self.progress.clone());
        let game = e.pick_game(folder.as_deref()).ok();
        result(Ok::<_, VmmError>(e.deploy_status(game.as_ref())))
    }

    #[tool(
        description = "Checks changes the user asked for and returns a plan: its steps in plain words and a \
                       planId. Changes nothing in the game. Show the user the steps before applying.",
        output_schema = schema_for_output::<Plan>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = false, open_world_hint = false)
    )]
    async fn plan_change(
        &self,
        Parameters(a): Parameters<PlanArgs>,
        peer: Peer<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let (e, folder) = engine_as(&a.game, Caller::Mcp { client: client_name(&peer) });
        result(e.pick_game(folder.as_deref()).and_then(|g| e.plan(&g, a.changes)))
    }

    #[tool(
        description = "Applies a plan from plan_change, once, if nothing it depends on has changed. Only when the \
                       user asked for this change; pass their request in their own words. Refused \
                       (ai.changes_off) unless the user allows changes from AI tools in Settings.",
        output_schema = schema_for_output::<ApplyReport>(),
        annotations(read_only_hint = false, destructive_hint = true, idempotent_hint = false, open_world_hint = false)
    )]
    async fn apply_plan(
        &self,
        Parameters(a): Parameters<ApplyArgs>,
        peer: Peer<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let caller = Caller::Mcp { client: client_name(&peer) };
        let (e, folder) = engine_as(&a.game, caller.clone());
        result(e.pick_game(folder.as_deref()).and_then(|g| e.apply_plan(&g, &a.plan_id, &caller, Some(&a.request))))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Vmm {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("vaexil-mod-manager", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }
}

/// Serves MCP on stdin and stdout until the client disconnects.
pub fn serve() -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|e| format!("Couldn't start the MCP server: {e}"))?;
    runtime.block_on(async {
        let service = Vmm::new()
            .serve(rmcp::transport::stdio())
            .await
            .map_err(|e| format!("The MCP client didn't connect: {e}"))?;
        service.waiting().await.map(|_| ()).map_err(|e| format!("The MCP server stopped: {e}"))
    })
}
