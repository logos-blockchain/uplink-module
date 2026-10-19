{
  description = "uplink_module: the Uplink node referral program over LEZ, through lez_core.";

  inputs = {
    logos-module-builder.url = "github:logos-co/logos-module-builder";

    # Dependency modules: their published .lidl contracts generate the typed
    # modules().lez_core / modules().lez_indexer_module clients.
    lez_core = {
      url = "github:logos-blockchain/logos-execution-zone-module?rev=dde621ab4dbd873d8a2b314932a9f2180d03813a";
      inputs.logos-module-builder.follows = "logos-module-builder";
    };
    lez_indexer_module = {
      url = "github:logos-blockchain/lez-indexer-module?rev=87463b1173fc685dd40f789d5de013c2564519da";
      inputs.logos-module-builder.follows = "logos-module-builder";
    };
  };

  outputs = inputs@{ logos-module-builder, ... }:
    let
      # The referral program binary comes from uplink at the commit referral_core is pinned to
      # in rust-lib/Cargo.lock, so the crate and the binary can never come from different commits.
      lock = builtins.fromTOML (builtins.readFile ./rust-lib/Cargo.lock);
      referralCore = builtins.head (builtins.filter (p: p.name == "referral_core") lock.package);
      uplinkCommit = builtins.elemAt (builtins.split "#" referralCore.source) 2;
      uplink = builtins.fetchGit {
        url = "https://github.com/logos-blockchain/uplink";
        rev = uplinkCommit;
        allRefs = true;
      };
    in
    logos-module-builder.lib.mkLogosModule {
      src = ./.;
      configFile = ./metadata.json;
      flakeInputs = inputs;
      rustEnv.UPLINK_REFERRAL_BIN = "${uplink}/artifacts/programs/referral/referral.bin";
    };
}
