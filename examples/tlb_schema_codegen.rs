use tonutils::tlb::schema;

fn main() -> anyhow::Result<()> {
    let user_schema = r#"
        demo_state$10 seqno:uint32 owner:bits256 = DemoState;
        demo_empty$0 = DemoMaybe;
    "#;
    let user_constructors = schema::parse_schema(user_schema)?;
    let user_summary = schema::generate_summary(&user_constructors);

    let phase1_constructors = schema::parse_schema(schema::BLOCK_PHASE1_TLB)?;
    let phase1_generated = schema::generate_block_phase1()?;

    println!("user_constructors={}", user_constructors.len());
    println!("user_summary={user_summary}");
    println!("phase1_constructors={}", phase1_constructors.len());
    println!(
        "phase1_checked_in_matches={}",
        phase1_generated == schema::BLOCK_PHASE1_GENERATED
    );
    Ok(())
}
