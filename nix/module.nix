{ config, lib, pkgs, ... }:

let
  cfg = config.services.noflex;
in
{
  options.services.noflex = {
    enable = lib.mkEnableOption "the NoFlex media server";

    package = lib.mkOption {
      type = lib.types.package;
      default = pkgs.noflex;
      defaultText = lib.literalExpression "pkgs.noflex";
      description = "The noflex package to use.";
    };

    user = lib.mkOption {
      type = lib.types.str;
      default = "noflex";
      description = "User account under which NoFlex runs.";
    };

    group = lib.mkOption {
      type = lib.types.str;
      default = "noflex";
      description = "Group under which NoFlex runs.";
    };

    bind = lib.mkOption {
      type = lib.types.str;
      default = "0.0.0.0:28793";
      description = "Address and port the server binds to.";
    };

    dataDir = lib.mkOption {
      type = lib.types.path;
      default = "/var/lib/noflex";
      description = "Directory for persistent data (database, embeddings).";
    };

    cacheDir = lib.mkOption {
      type = lib.types.path;
      default = "/var/cache/noflex";
      description = "Directory for transient caches (transcodes, images).";
    };

    mediaPaths = lib.mkOption {
      type = lib.types.listOf lib.types.path;
      default = [ ];
      example = [ "/media/movies" "/media/shows" "/media/music" ];
      description = "Media directories the user has granted read access to.";
    };

    configFile = lib.mkOption {
      type = lib.types.path;
      default = "/etc/noflex/config.toml";
      description = "Path to the noflex configuration file.";
    };

    openFirewall = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Open the configured port in the firewall.";
    };

    logFile = lib.mkOption {
      type = lib.types.nullOr lib.types.path;
      default = null;
      example = "/var/log/noflex/noflex.log";
      description = "Optional path to the log file (rotation planned).";
    };

    hwaccel = lib.mkOption {
      type = lib.types.enum [ "none" "auto" "vaapi" "videotoolbox" "nvenc" "qsv" "amf" "mf" ];
      default = "auto";
      description = "Hardware-accelerated transcode backend.";
    };

    pluginPath = lib.mkOption {
      type = lib.types.listOf lib.types.path;
      default = [ ];
      example = [ "/etc/noflex/plugins" "/var/lib/noflex/plugins" ];
      description = "Directories the plugin host scans on startup.";
    };

    backupDir = lib.mkOption {
      type = lib.types.nullOr lib.types.path;
      default = null;
      example = "/var/backups/noflex";
      description = "If set, run scheduled backups into this directory.";
    };

    backupIntervalHours = lib.mkOption {
      type = lib.types.int;
      default = 0;
      description = "Hours between scheduled backups. 0 = run once at startup.";
    };

    backupKeep = lib.mkOption {
      type = lib.types.int;
      default = 7;
      description = "How many old backups to keep on disk.";
    };
  };

  config = lib.mkIf cfg.enable {
    users.users.${cfg.user} = {
      isSystemUser = true;
      group = cfg.group;
      home = cfg.dataDir;
    };
    users.groups.${cfg.group} = { };

    systemd.tmpfiles.rules = [
      "d ${cfg.dataDir}  0750 ${cfg.user} ${cfg.group} -"
      "d ${cfg.cacheDir} 0750 ${cfg.user} ${cfg.group} -"
    ] ++ lib.optional (cfg.backupDir != null)
      "d ${toString cfg.backupDir} 0750 ${cfg.user} ${cfg.group} -"
    ++ lib.optional (cfg.logFile != null)
      "d ${dirOf (toString cfg.logFile)} 0750 ${cfg.user} ${cfg.group} -";

    systemd.services.noflex = {
      description = "NoFlex media server";
      wantedBy = [ "multi-user.target" ];
      after = [ "network-online.target" ];
      wants = [ "network-online.target" ];

      # Names from ENV_KEYS in noflex-server's config.rs; a test there fails
      # on any it doesn't read.
      environment = {
        NOFLEX_BIND = cfg.bind;
        NOFLEX_DATA_DIR = toString cfg.dataDir;
        NOFLEX_CACHE_DIR = toString cfg.cacheDir;
        NOFLEX_TRANSCODER_HWACCEL = cfg.hwaccel;
        NOFLEX_CONFIG_FILE = toString cfg.configFile;
      } // lib.optionalAttrs (cfg.pluginPath != [ ]) {
        NOFLEX_PLUGIN_SEARCH_PATH = lib.concatStringsSep ":" (map toString cfg.pluginPath);
      } // lib.optionalAttrs (cfg.logFile != null) {
        NOFLEX_LOG_FILE = toString cfg.logFile;
      } // lib.optionalAttrs (cfg.backupDir != null) {
        NOFLEX_BACKUP_DIR = toString cfg.backupDir;
        NOFLEX_BACKUP_INTERVAL_HOURS = toString cfg.backupIntervalHours;
        NOFLEX_BACKUP_KEEP = toString cfg.backupKeep;
      };
      serviceConfig = {
        ExecStart = "${lib.getExe cfg.package}";
        User = cfg.user;
        Group = cfg.group;
        Restart = "on-failure";
        RestartSec = "5s";
        # VAAPI and QSV open the GPU's render node.
        SupplementaryGroups = [ "render" "video" ];

        # Hardening.
        ProtectSystem = "strict";
        ProtectHome = true;
        PrivateTmp = true;
        NoNewPrivileges = true;
        ReadWritePaths = [ cfg.dataDir cfg.cacheDir ]
          ++ lib.optional (cfg.backupDir != null) (toString cfg.backupDir)
          ++ lib.optional (cfg.logFile != null) (dirOf (toString cfg.logFile));
        ReadOnlyPaths = cfg.mediaPaths;
        StateDirectory = baseNameOf cfg.dataDir;
        CacheDirectory = baseNameOf cfg.cacheDir;
      };
    };

    networking.firewall = lib.mkIf cfg.openFirewall {
      allowedTCPPorts = [
        (lib.toInt (lib.last (lib.splitString ":" cfg.bind)))
      ];
    };
  };
}
