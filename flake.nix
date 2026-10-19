{
  description = "Logos referral module: the node referral programme over LEZ, through lez_core.";

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
    logos-module-builder.lib.mkLogosModule {
      src = ./.;
      configFile = ./metadata.json;
      flakeInputs = inputs;
    };
}
