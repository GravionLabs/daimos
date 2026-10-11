//! Prints what the application source finds on this machine: `cargo run --example scan_apps`.

fn main() {
    #[cfg(target_os = "linux")]
    {
        use daimos_core::apps::AppSource;
        let started = std::time::Instant::now();
        let apps = daimos_platform::apps::linux::LinuxAppSource::new().scan();
        println!("{} apps in {:?}", apps.len(), started.elapsed());
        for app in apps {
            println!(
                "{:<32} {:<10} actions={} icon={}",
                app.name,
                if app.terminal { "terminal" } else { "" },
                app.actions.len(),
                app.icon.map_or("-".into(), |i| i.display().to_string())
            );
        }
    }
}
