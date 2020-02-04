// Copyright 2018-2019 Parity Technologies (UK) Ltd. and Centrality Investments Ltd.
// This file is part of Substrate.

// Substrate is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// Substrate is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with Substrate.  If not, see <http://www.gnu.org/licenses/>.

//! Substrate CLI library.

#![warn(missing_docs)]
#![warn(unused_extern_crates)]

pub use sc_cli::error;
#[macro_use]
mod chain_spec;
mod factory_impl;
mod service;

use crate::factory_impl::FactoryState;
use log::info;
use node_transaction_factory::RuntimeAdapter;
use sc_cli::{parse_and_prepare, AugmentClap, GetLogFilter, ParseAndPrepare};
pub use sc_cli::{ExecutionStrategyParam, IntoExit, NoCustom, SharedParams, VersionInfo};
use sc_client::ExecutionStrategies;
use sc_service::{AbstractService, Configuration, Roles as ServiceRoles};
use structopt::{clap::App, StructOpt};
use tokio::prelude::Future;
use tokio::runtime::{Builder as RuntimeBuilder, Runtime};

/// The chain specification option.
#[derive(Clone, Debug, PartialEq)]
pub enum ChainSpec {
	/// Whatever the current runtime is, with just Alice as an auth.
	Development,
	/// The CENNZnet Kauri testnet.
	CennznetKauri,
	/// The CENNZnet Rimu testnet.
	CennznetRimu,
}

/// Custom subcommands.
#[derive(Clone, Debug, StructOpt)]
pub enum CustomSubcommands {
	/// The custom factory subcommmand for manufacturing transactions.
	#[structopt(
		name = "factory",
		about = "Manufactures num transactions from Alice to random accounts. \
		         Only supported for development or local testnet."
	)]
	Factory(FactoryCmd),
}

impl GetLogFilter for CustomSubcommands {
	fn get_log_filter(&self) -> Option<String> {
		None
	}
}

/// The `factory` command used to generate transactions.
/// Please note: this command currently only works on an empty database!
#[derive(Debug, StructOpt, Clone)]
pub struct FactoryCmd {
	/// How often to repeat. This option only has an effect in mode `MasterToNToM`.
	#[structopt(long = "rounds", default_value = "1")]
	pub rounds: u64,

	/// MasterToN: Manufacture `num` transactions from the master account
	///            to `num` randomly created accounts, one each.
	///
	/// MasterTo1: Manufacture `num` transactions from the master account
	///            to exactly one other randomly created account.
	///
	/// MasterToNToM: Manufacture `num` transactions from the master account
	///               to `num` randomly created accounts.
	///               From each of these randomly created accounts manufacture
	///               a transaction to another randomly created account.
	///               Repeat this `rounds` times. If `rounds` = 1 the behavior
	///               is the same as `MasterToN`.{n}
	///               A -> B, A -> C, A -> D, ... x `num`{n}
	///               B -> E, C -> F, D -> G, ...{n}
	///               ... x `rounds`
	///
	/// These three modes control manufacturing.
	#[structopt(long = "mode", default_value = "MasterToN")]
	pub mode: node_transaction_factory::Mode,

	/// Number of transactions to generate. In mode `MasterNToNToM` this is
	/// the number of transactions per round.
	#[structopt(long = "num", default_value = "8")]
	pub num: u64,

	#[allow(missing_docs)]
	#[structopt(flatten)]
	pub shared_params: SharedParams,

	/// The means of execution used when calling into the runtime while importing blocks.
	#[structopt(
		long = "execution",
		value_name = "STRATEGY",
		possible_values = &ExecutionStrategyParam::variants(),
		case_insensitive = true,
		default_value = "NativeElseWasm"
	)]
	pub execution: ExecutionStrategyParam,
}

impl AugmentClap for FactoryCmd {
	fn augment_clap<'a, 'b>(app: App<'a, 'b>) -> App<'a, 'b> {
		FactoryCmd::augment_clap(app)
	}
}

/// Get a chain config from a spec setting.
impl ChainSpec {
	pub(crate) fn load(self) -> Result<sc_chain_spec::ChainSpec, String> {
		Ok(match self {
			ChainSpec::Development => sc_chain_spec::dev::config(),
			ChainSpec::CennznetKauri => sc_chain_spec::kauri::config(),
			ChainSpec::CennznetRimu => sc_chain_spec::rimu::config(),
		})
	}

	pub(crate) fn from(s: &str) -> Option<Self> {
		match s {
			"dev" => Some(ChainSpec::Development),
			"kauri" => Some(ChainSpec::CennznetKauri),
			"rimu" => Some(ChainSpec::CennznetRimu),
			_ => None,
		}
	}
}

fn load_spec(id: &str) -> Result<Option<sc_chain_spec::ChainSpec>, String> {
	Ok(match ChainSpec::from(id) {
		Some(spec) => Some(spec.load()?),
		None => None,
	})
}

/// Parse command line arguments into service configuration.
pub fn run<I, T, E>(args: I, exit: E, version: sc_cli::VersionInfo) -> error::Result<()>
where
	I: IntoIterator<Item = T>,
	T: Into<std::ffi::OsString> + Clone,
	E: IntoExit,
{
	type Config<A, B> = Configuration<(), A, B>;

	match parse_and_prepare::<CustomSubcommands, NoCustom, _>(&version, "cennznet-node", args) {
		ParseAndPrepare::Run(cmd) => cmd.run(
			load_spec,
			exit,
			|exit, _cli_args, _custom_args, config: Config<_, _>| {
				info!("{}", version.name);
				info!("  version {}", config.full_version());
				info!("  by Centrality Investments UNlimited, 2018-2019");
				info!("Chain specification: {}", config.chain_spec.name());
				info!("Node name: {}", config.name);
				info!("Roles: {:?}", config.roles);
				let runtime = RuntimeBuilder::new()
					.name_prefix("main-tokio-")
					.build()
					.map_err(|e| format!("{:?}", e))?;
				match config.roles {
					ServiceRoles::LIGHT => run_until_exit(runtime, service::new_light(config)?, exit),
					_ => run_until_exit(runtime, service::new_full(config)?, exit),
				}
			},
		),
		ParseAndPrepare::BuildSpec(cmd) => cmd.run(load_spec),
		ParseAndPrepare::ExportBlocks(cmd) => {
			cmd.run_with_builder(|config: Config<_, _>| Ok(new_full_start!(config).0), load_spec, exit)
		}
		ParseAndPrepare::ImportBlocks(cmd) => {
			cmd.run_with_builder(|config: Config<_, _>| Ok(new_full_start!(config).0), load_spec, exit)
		}
		ParseAndPrepare::PurgeChain(cmd) => cmd.run(load_spec),
		ParseAndPrepare::RevertChain(cmd) => {
			cmd.run_with_builder(|config: Config<_, _>| Ok(new_full_start!(config).0), load_spec)
		}
		ParseAndPrepare::CustomCommand(CustomSubcommands::Factory(cli_args)) => {
			let mut config: Config<_, _> =
				sc_cli::create_config_with_db_path(load_spec, &cli_args.shared_params, &version)?;
			config.execution_strategies = ExecutionStrategies {
				importing: cli_args.execution.into(),
				block_construction: cli_args.execution.into(),
				other: cli_args.execution.into(),
				..Default::default()
			};

			match ChainSpec::from(config.chain_spec.id()) {
				Some(ref c)
					if c == &ChainSpec::Development
						|| c == &ChainSpec::CennznetKauri
						|| c == &ChainSpec::CennznetRimu => {}
				_ => panic!("Factory is only supported for dev, kauri and rimu"),
			}

			let factory_state = FactoryState::new(cli_args.mode.clone(), cli_args.num, cli_args.rounds);

			let service_builder = new_full_start!(config).0;
			node_transaction_factory::factory::<FactoryState<_>, _, _, _, _, _>(
				factory_state,
				service_builder.client(),
				service_builder
					.select_chain()
					.expect("The select_chain is always initialized by new_full_start!; QED"),
			)
			.map_err(|e| format!("Error in transaction factory: {}", e))?;

			Ok(())
		}
	}
}

fn run_until_exit<T, E>(mut runtime: Runtime, service: T, e: E) -> error::Result<()>
where
	T: AbstractService,
	E: IntoExit,
{
	let (exit_send, exit) = exit_future::signal();

	let informant = sc_cli::informant::build(&service);
	runtime.executor().spawn(exit.until(informant).map(|_| ()));

	// we eagerly drop the service so that the internal exit future is fired,
	// but we need to keep holding a reference to the global telemetry guard
	let _telemetry = service.telemetry();

	let service_res = {
		let exit = e
			.into_exit()
			.map_err(|_| error::Error::Other("Exit future failed.".into()));
		let service = service.map_err(|err| error::Error::Service(err));
		let select = service.select(exit).map(|_| ()).map_err(|(err, _)| err);
		runtime.block_on(select)
	};

	exit_send.fire();

	// TODO [andre]: timeout this future #1318
	let _ = runtime.shutdown_on_idle().wait();

	service_res
}