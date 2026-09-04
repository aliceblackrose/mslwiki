use msl_db::{AstromonRecord, CatalogSnapshot, Database, ItemRecord, SkillRecord};

#[tokio::test]
async fn imports_and_searches_catalog_records() {
    let temp = tempfile::tempdir().unwrap();
    let database = Database::connect(&temp.path().join("wiki.sqlite"))
        .await
        .unwrap();

    let catalog = CatalogSnapshot {
        astromons: vec![AstromonRecord {
            id: 101,
            slug: "fire-mona".to_owned(),
            name: "Mona".to_owned(),
            element: "Fire".to_owned(),
            rarity: Some(4),
            variant: Some("Variant".to_owned()),
            role: Some("Attacker".to_owned()),
            hp: Some(1200),
            attack: Some(930),
            defense: Some(610),
            recovery: Some(720),
            skill_one_id: Some(201),
            skill_two_id: None,
            leader_skill_id: None,
            portrait_asset: Some("assets/sprites/mona.png".to_owned()),
        }],
        skills: vec![SkillRecord {
            id: 201,
            slug: "flame-strike".to_owned(),
            name: "Flame Strike".to_owned(),
            description: "Deals fire damage.".to_owned(),
            skill_type: Some("Active".to_owned()),
            element: Some("Fire".to_owned()),
            cooldown: Some(3),
            icon_asset: Some("assets/sprites/flame-strike.png".to_owned()),
        }],
        items: vec![ItemRecord {
            id: 301,
            slug: "test-gem".to_owned(),
            name: "Test Gem".to_owned(),
            description: "A test item.".to_owned(),
            item_type: Some("Gem".to_owned()),
            rarity: Some(3),
            icon_asset: None,
        }],
    };

    let counts = database.import_catalog(&catalog).await.unwrap();
    assert_eq!(counts.astromons, 1);
    assert_eq!(counts.skills, 1);
    assert_eq!(counts.items, 1);

    let astromons = database
        .astromons("mon", "fire", Some(4), 48, 0)
        .await
        .unwrap();
    assert_eq!(astromons.len(), 1);
    assert_eq!(astromons[0].slug, "fire-mona");

    let by_slug = database.astromon("fire-mona").await.unwrap().unwrap();
    assert_eq!(by_slug.id, 101);
    assert_eq!(by_slug.skill_one_id, Some(201));

    let by_id = database.astromon("101").await.unwrap().unwrap();
    assert_eq!(by_id.name, "Mona");

    let results = database.search_catalog("flame", None, 20).await.unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].kind, "skill");
    assert_eq!(results[0].slug, "flame-strike");

    let items = database.items("gem", 20, 0).await.unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, 301);
}
