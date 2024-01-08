use tendermint::{
    abci::{request::BeginBlock, types::CommitInfo},
    account,
    block::{Header, Height, Round},
    chain,
    validator::Set,
    AppHash, Hash, Time,
};
use tendermint_light_client_verifier::{
    options::Options,
    types::{TrustedBlockState, UntrustedBlockState},
    Verdict, Verifier,
};

pub fn begin_block() -> BeginBlock {
    BeginBlock {
        hash: Hash::None,
        header: header(),
        last_commit_info: CommitInfo {
            round: Round::default(),
            votes: vec![],
        },
        byzantine_validators: vec![],
    }
}

fn header() -> Header {
    use tendermint::block::header::Version;
    Header {
        version: Version { block: 0, app: 0 },
        chain_id: chain::Id::try_from("test").unwrap(),
        height: Height::default(),
        time: Time::unix_epoch(),
        last_block_id: None,
        last_commit_hash: None,
        data_hash: None,
        validators_hash: validators().hash(),
        next_validators_hash: validators().hash(),
        consensus_hash: Hash::None,
        app_hash: app_hash(),
        last_results_hash: None,
        evidence_hash: None,
        proposer_address: account::Id::new([
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ]),
    }
}

fn validators() -> Set {
    Set::new(vec![], None)
}

fn app_hash() -> AppHash {
    AppHash::try_from(vec![1, 2, 3]).unwrap()
    // AppHash::try_from is infallible, see: https://github.com/informalsystems/tendermint-rs/issues/1243
}

pub struct TestVerifier;
impl Verifier for TestVerifier {
    fn verify_update_header(
        &self,
        _untrusted: UntrustedBlockState<'_>,
        _trusted: TrustedBlockState<'_>,
        _options: &Options,
        _now: Time,
    ) -> Verdict {
        todo!("<BarkBark as Verifier>::verify_update_header")
    }
    fn verify_misbehaviour_header(
        &self,
        _untrusted: UntrustedBlockState<'_>,
        _trusted: TrustedBlockState<'_>,
        _options: &Options,
        _now: Time,
    ) -> Verdict {
        todo!("<BarkBark as Verifier>::verify_misbehaviour_header")
    }
}

#[cfg(test)]
mod tests {
    // use tendermint_light_client_verifier::
    use cnidarium::{ArcStateDeltaExt, StateDelta, TempStorage};
    use cnidarium_component::{ActionHandler as _, Component};
    use penumbra_app::{MockClient, TempStorageExt};
    use penumbra_chain::component::StateWriteExt;
    use penumbra_compact_block::component::CompactBlockManager;
    use penumbra_keys::{test_keys, PayloadKey};
    use penumbra_sct::component::SourceContext;
    use penumbra_shielded_pool::{component::ShieldedPool, SpendPlan};
    use penumbra_txhash::{AuthorizingData, EffectHash, TransactionContext};
    use rand_core::SeedableRng;
    use std::{ops::Deref, sync::Arc};
    use tendermint::abci;

    #[test]
    fn begin_block_works() {
        let _ = super::begin_block();
        // next, parse this block via a light client
    }

    // XXX(kate): copied from `crates/core/app/src/tests/spend.rs`
    #[tokio::test]
    async fn spend_happy_path() -> anyhow::Result<()> {
        let mut rng = rand_chacha::ChaChaRng::seed_from_u64(1312);

        let storage = TempStorage::new().await?.apply_default_genesis().await?;
        let mut state = Arc::new(StateDelta::new(storage.latest_snapshot()));

        let height = 1;

        // Precondition: This test uses the default genesis which has existing notes for the test keys.
        let mut client = MockClient::new(test_keys::FULL_VIEWING_KEY.clone());
        let sk = test_keys::SPEND_KEY.clone();
        client.sync_to(0, state.deref()).await?;
        let note = client.notes.values().next().unwrap().clone();
        let note_commitment = note.commit();
        let proof = client.sct.witness(note_commitment).unwrap();
        let root = client.sct.root();
        let tct_position = proof.position();

        // 1. Simulate BeginBlock
        let mut state_tx = state.try_begin_transaction().unwrap();
        state_tx.put_block_height(height);
        state_tx.put_epoch_by_height(
            height,
            penumbra_chain::Epoch {
                index: 0,
                start_height: 0,
            },
        );
        state_tx.apply();

        // 2. Create a Spend action
        let spend_plan = SpendPlan::new(&mut rng, note, tct_position);
        let dummy_effect_hash = [0u8; 64];
        let rsk = sk.spend_auth_key().randomize(&spend_plan.randomizer);
        let auth_sig = rsk.sign(&mut rng, dummy_effect_hash.as_ref());
        let spend = spend_plan.spend(&test_keys::FULL_VIEWING_KEY, auth_sig, proof, root);
        let transaction_context = TransactionContext {
            anchor: root,
            effect_hash: EffectHash(dummy_effect_hash),
        };

        // 3. Simulate execution of the Spend action
        spend.check_stateless(transaction_context).await?;
        spend.check_stateful(state.clone()).await?;
        let mut state_tx = state.try_begin_transaction().unwrap();
        state_tx.put_mock_source(1u8);
        spend.execute(&mut state_tx).await?;
        state_tx.apply();

        // 4. Execute EndBlock

        let end_block = abci::request::EndBlock {
            height: height.try_into().unwrap(),
        };
        ShieldedPool::end_block(&mut state, &end_block).await;

        let mut state_tx = state.try_begin_transaction().unwrap();
        // ... and for the App, call `finish_block` to correctly write out the SCT with the data we'll use next.
        state_tx.finish_block(false).await.unwrap();

        state_tx.apply();

        Ok(())
    }
}
