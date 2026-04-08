pub mod router;
pub mod agent;
pub mod agent_loop;
pub mod coordinator;
pub mod consensus;
pub mod compact;

pub use router::{SmartRouter, TaskCategory};
pub use agent::ReactAgent;
pub use agent_loop::{AgentEvent, AgentCommand, run_agent_loop};
pub use coordinator::Coordinator;
pub use consensus::{Council, CouncilVerdict, ConsensusLevel};
pub use compact::{context_limit, needs_compression, compression_status};
