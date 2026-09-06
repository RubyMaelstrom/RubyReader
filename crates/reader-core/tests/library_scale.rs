use reader_core::{db::Store, model::*};

/// Run with `cargo test -p reader-core --release --test library_scale -- --ignored --nocapture`.
/// Uses a real on-disk SQLite database and the same public queries as the UI.
#[test]
#[ignore = "50,000-article performance acceptance"]
fn five_hundred_feeds_fifty_thousand_articles() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("library.sqlite");
    let mut db = Store::open(&path)?;
    let start = std::time::Instant::now();
    for f in 0..500 {
        let id = db.add_feed(
            &format!("https://feed-{f}.example.test/rss"),
            &format!("Feed {f:03}"),
            None,
        )?;
        let items=(0..100).map(|i|NewArticle{identity:format!("{f}-{i}"),title:format!("Field notes {f}-{i}"),url:format!("https://feed-{f}.example.test/{i}"),author:"Writer".into(),published:now()-i*3600,summary:"A botanical field note".into(),content:format!("<h2>Notes from the garden</h2><p>Orchid observations number {i}, with detailed seasonal notes, illustrations, and a searchable uncommonword.</p>")}).collect::<Vec<_>>();
        db.update_articles(id, &items)?;
    }
    println!("Seeded 50,000 articles in {:?}", start.elapsed());
    let query = Query {
        search: "uncommonword".into(),
        ..Default::default()
    };
    assert_eq!(db.snapshot(&query)?.total, 50_000);
    let mut timings = Vec::new();
    for _ in 0..5 {
        let start = std::time::Instant::now();
        let snapshot = db.snapshot(&query)?;
        assert_eq!(snapshot.feeds.len(), 500);
        assert_eq!(snapshot.articles.len(), 150);
        timings.push(start.elapsed());
    }
    println!("Warm full-text search + sidebar + first page: {timings:?}");
    assert!(
        timings.iter().all(|t| t.as_millis() < 200),
        "Search should remain below the 200ms target"
    );
    drop(db);
    let db = Store::open(&path)?;
    assert_eq!(db.snapshot(&query)?.total, 50_000);
    Ok(())
}
