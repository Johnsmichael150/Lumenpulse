// Invariant test suite for crowdfund_vault
// Feature: invariant-hardening
//
// Each property test references the requirement it validates via a comment.
// All properties run 1000 cases (ProptestConfig::with_cases(1000)).

use crowdfund_vault::CrowdfundVaultContractClient;
use proptest::prelude::*;
use soroban_sdk::{
    symbol_short,
    testutils::Address as _,
    token::{StellarAssetClient, TokenClient},
    Address, Env,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// All handles needed to interact with a freshly-initialised contract.
struct TestContext<'a> {
    env: Env,
    client: CrowdfundVaultContractClient<'a>,
    admin: Address,
    owner: Address,
    token: TokenClient<'a>,
    token_admin: StellarAssetClient<'a>,
    project_id: u64,
}

/// Create a fresh Soroban Env, register the contract, initialise it, mint
/// tokens to `owner`, and create one project.  Returns a `TestContext` with
/// all handles wired up.
fn setup_env<'a>(env: &'a Env) -> TestContext<'a> {
    env.mock_all_auths();

    let admin = Address::generate(env);
    let owner = Address::generate(env);

    // Register a Stellar asset contract so we have a real token.
    let asset_contract = env.register_stellar_asset_contract_v2(admin.clone());
    let token = TokenClient::new(env, &asset_contract.address());
    let token_admin = StellarAssetClient::new(env, &asset_contract.address());

    // Mint a generous supply to the owner so deposits can succeed.
    token_admin.mint(&owner, &100_000_000);

    // Register and initialise the vault contract.
    let contract_id = env.register(crowdfund_vault::CrowdfundVaultContract, ());
    let client = CrowdfundVaultContractClient::new(env, &contract_id);
    client.initialize(&admin);

    // Create one project owned by `owner`.
    let project_id = client.create_project(
        &owner,
        &symbol_short!("TestProj"),
        &1_000_000,
        &token.address,
    );

    TestContext {
        env: env.clone(),
        client,
        admin,
        owner,
        token,
        token_admin,
        project_id,
    }
}

// ---------------------------------------------------------------------------
// Placeholder — properties will be added in subsequent tasks
// ---------------------------------------------------------------------------

#[test]
fn scaffold_compiles() {
    let env = Env::default();
    let _ctx = setup_env(&env);
    // No assertions — just verifies the scaffold compiles and setup_env works.
}

// ---------------------------------------------------------------------------
// Balance Conservation — Requirement 1
// ---------------------------------------------------------------------------

// Feature: invariant-hardening, Property 1: Balance equals sum of contributions
// Validates: Requirements 1.1, 1.5
proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    #[test]
    fn prop_balance_equals_sum_of_contributions(
        amounts in proptest::collection::vec(1i128..=1_000_000i128, 1..=5usize)
    ) {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup_env(&env);

        // Generate a distinct contributor address for each amount and deposit.
        for amount in &amounts {
            let contributor = Address::generate(&env);
            // Mint enough tokens to this contributor.
            ctx.token_admin.mint(&contributor, amount);
            ctx.client.deposit(&contributor, &ctx.project_id, amount);
        }

        let expected_sum: i128 = amounts.iter().sum();

        // ProjectBalance must equal the sum of all deposits.
        let balance = ctx.client.get_balance(&ctx.project_id);
        prop_assert_eq!(balance, expected_sum);

        // total_deposited on ProjectData must also equal the sum.
        let project = ctx.client.get_project(&ctx.project_id);
        prop_assert_eq!(project.total_deposited, expected_sum);
    }
}

// ---------------------------------------------------------------------------
// Withdrawal Safety — Requirement 3
// ---------------------------------------------------------------------------

// Feature: invariant-hardening, Property 7: Overdraft is rejected
// Validates: Requirements 3.1
proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    #[test]
    fn prop_overdraft(
        deposit_amount in 1i128..=1_000_000i128,
        excess in 1i128..=1_000_000i128,
    ) {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup_env(&env);

        // Deposit D tokens from the owner into the project.
        ctx.token_admin.mint(&ctx.owner, &deposit_amount);
        ctx.client.deposit(&ctx.owner, &ctx.project_id, &deposit_amount);

        // Approve the milestone so the withdrawal gate is open.
        ctx.client.approve_milestone(&ctx.admin, &ctx.project_id);

        // Attempt to withdraw W = D + excess, which exceeds the balance.
        let withdraw_amount = deposit_amount + excess;
        let result = ctx.client.try_withdraw(&ctx.project_id, &withdraw_amount);

        // Must be rejected with InsufficientBalance.
        prop_assert!(
            result.is_err(),
            "expected InsufficientBalance error but got Ok"
        );

        // Balance must remain unchanged at D.
        let balance = ctx.client.get_balance(&ctx.project_id);
        prop_assert_eq!(balance, deposit_amount);
    }
}

// ---------------------------------------------------------------------------
// Quadratic Funding Math — Requirement 4
// ---------------------------------------------------------------------------

// Feature: invariant-hardening, Property 9: Single-contributor match approximation
// Validates: Requirements 4.2
proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    #[test]
    fn prop_single_contributor_match(c in 1i128..=10_000_000i128) {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup_env(&env);

        let contributor = Address::generate(&env);
        ctx.token_admin.mint(&contributor, &c);
        ctx.client.deposit(&contributor, &ctx.project_id, &c);

        let match_amount = ctx.client.calculate_match(&ctx.project_id);

        // For a single contributor with amount C, the quadratic formula gives
        // (sqrt(C))^2 = C, so the result should be approximately C within 1%
        // tolerance for fixed-point rounding in sqrt_scaled.
        let lower = c * 99 / 100;
        let upper = c * 101 / 100;

        prop_assert!(
            match_amount >= lower && match_amount <= upper,
            "calculate_match({}) = {} is outside [{}, {}]",
            c, match_amount, lower, upper
        );
    }
}

// ---------------------------------------------------------------------------
// Pause Invariants — Requirement 5
// ---------------------------------------------------------------------------

// Feature: invariant-hardening, Property 11: Paused contract blocks all mutations
// Validates: Requirements 5.1, 5.2, 5.3
proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    #[test]
    fn prop_paused_blocks_mutations(
        deposit_amount in 1i128..=1_000_000i128,
        target_amount in 1i128..=1_000_000i128,
    ) {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup_env(&env);

        // Pause the contract.
        ctx.client.pause(&ctx.admin);

        // --- try_deposit must return ContractPaused ---
        let contributor = Address::generate(&env);
        ctx.token_admin.mint(&contributor, &deposit_amount);
        let deposit_result = ctx.client.try_deposit(&contributor, &ctx.project_id, &deposit_amount);
        prop_assert!(
            matches!(
                deposit_result,
                Err(Ok(crowdfund_vault::CrowdfundError::ContractPaused))
            ),
            "try_deposit while paused should return ContractPaused, got {:?}",
            deposit_result
        );

        // Balance must remain 0 (no state change).
        let balance = ctx.client.get_balance(&ctx.project_id);
        prop_assert_eq!(balance, 0i128, "balance should remain 0 after rejected deposit");

        // --- try_create_project must return ContractPaused ---
        let new_owner = Address::generate(&env);
        let create_result = ctx.client.try_create_project(
            &new_owner,
            &symbol_short!("NewProj"),
            &target_amount,
            &ctx.token.address,
        );
        prop_assert!(
            matches!(
                create_result,
                Err(Ok(crowdfund_vault::CrowdfundError::ContractPaused))
            ),
            "try_create_project while paused should return ContractPaused, got {:?}",
            create_result
        );

        // --- try_withdraw must return ContractPaused ---
        // (milestone approval also blocked while paused, but withdraw checks pause first)
        let withdraw_result = ctx.client.try_withdraw(&ctx.project_id, &1i128);
        prop_assert!(
            matches!(
                withdraw_result,
                Err(Ok(crowdfund_vault::CrowdfundError::ContractPaused))
            ),
            "try_withdraw while paused should return ContractPaused, got {:?}",
            withdraw_result
        );
    }
}

// ---------------------------------------------------------------------------
// Deposit Integrity — Requirement 2
// ---------------------------------------------------------------------------

// Feature: invariant-hardening, Property 4: Contribution tracking round-trip
// Validates: Requirements 2.1
proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    #[test]
    fn prop_contribution_tracking(amount in 1i128..=10_000_000i128) {
        let env = Env::default();
        env.mock_all_auths();
        let ctx = setup_env(&env);

        let contributor = Address::generate(&env);

        // Record prior contribution (should be 0 for a fresh contributor).
        let prior = ctx.client.get_contribution(&ctx.project_id, &contributor);

        // Mint enough tokens and deposit.
        ctx.token_admin.mint(&contributor, &amount);
        ctx.client.deposit(&contributor, &ctx.project_id, &amount);

        // After deposit, contribution must equal prior + amount.
        let after = ctx.client.get_contribution(&ctx.project_id, &contributor);
        prop_assert_eq!(after, prior + amount);
    }
}
