//! Calculate a reviewed Runtime admission identity from exact carrier files.

use apxm_runtime_service::{
    RuntimeAdmissionProfile, RuntimeCapabilityProfile, canonical_resource_ceiling_digest,
    port_bindings_digest_for,
};

fn main() {
    let result = (|| {
        let selected = RuntimeCapabilityProfile::from_env()?;
        let profile = RuntimeAdmissionProfile::from_env_for(selected)?
            .ok_or("both admission profile carriers are required")?;
        Ok::<_, String>(serde_json::json!({
            "profile_ref": profile.profile_ref(),
            "capability_profile": match selected {
                RuntimeCapabilityProfile::PortableLocal => "portable_local",
                RuntimeCapabilityProfile::HostOnly => "host_only",
            },
            "port_bindings_digest": port_bindings_digest_for(selected),
            "resource_ceiling_digest": canonical_resource_ceiling_digest(),
        }))
    })();
    match result {
        Ok(value) => println!("{value}"),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
