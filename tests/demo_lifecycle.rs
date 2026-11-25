#[cfg(target_os = "linux")]
mod linux {
    use pg_debugger::{demo_linux::DemoServer, pg_process};
    use postgres::{Client, NoTls};
    use std::{path::PathBuf, process::Command};

    #[test]
    #[ignore = "requires local PostgreSQL with pldebugger and a non-root user"]
    fn externally_stopped_demo_can_be_closed_twice() {
        let (mut server, _) = DemoServer::start_isolated().unwrap();
        let mut client = Client::connect(server.connection_string(), NoTls).unwrap();
        let data: String = client.query_one("SHOW data_directory", &[]).unwrap().get(0);
        let version: i32 = client
            .query_one("SELECT current_setting('server_version_num')::int", &[])
            .unwrap()
            .get(0);
        assert!(version >= 170000);
        let root = PathBuf::from(&data).parent().unwrap().to_owned();
        drop(client);
        let ctl = std::env::var_os("PGDEBUG_PG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/usr/lib/postgresql/17"))
            .join("bin/pg_ctl");
        assert!(
            Command::new(&ctl)
                .args(["-D", &data, "-m", "fast", "-w", "stop"])
                .status()
                .unwrap()
                .success()
        );
        server.shutdown().unwrap();
        assert!(!root.exists());
        server.shutdown().unwrap();

        let invalid = std::env::temp_dir().join(format!(
            "pg-debugger-invalid-status-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&invalid).unwrap();
        std::fs::write(invalid.join("marker"), "keep").unwrap();
        assert!(pg_process::stop(&ctl, &invalid).is_err());
        assert!(invalid.join("marker").exists());
        std::fs::remove_dir_all(invalid).unwrap();
    }
}
