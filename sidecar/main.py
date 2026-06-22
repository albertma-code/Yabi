"""PyInstaller entry point — invoked as the single-file Mach-O binary."""
from bilio_sidecar.rpc import main


if __name__ == "__main__":
    main()
