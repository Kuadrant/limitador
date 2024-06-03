use goose::prelude::*;

async fn loadtest_get_json(user: &mut GooseUser) -> TransactionResult {
    let _goose_metrics = user.get("/json").await?;

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), GooseError> {
    GooseAttack::initialize()?
        .register_scenario(
            scenario!("LoadtestTransactions").register_transaction(transaction!(loadtest_get_json)),
        )
        .set_default(GooseDefault::Host, "http://localhost:18000")?
        .set_default(GooseDefault::HatchRate, "2")?
        .set_default(
            GooseDefault::CoordinatedOmissionMitigation,
            GooseCoordinatedOmissionMitigation::Average,
        )?
        .set_default(GooseDefault::RunTime, 20)?
        .execute()
        .await?;

    Ok(())
}
