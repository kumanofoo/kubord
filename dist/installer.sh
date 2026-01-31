#!/bin/bash

set -e

# ============================================================
# Project Configuration
# Change PROJECT_NAME to customize installation paths and service names
# Change ENV_FILE to customize environment variable filename
# ============================================================
PROJECT_NAME="kubord"
ENV_FILE="kubord"

# Configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TARGET_DIR="/opt/${PROJECT_NAME}"
BIN_DIR="${TARGET_DIR}/bin"
ETC_DIR="${TARGET_DIR}/etc"
ENV_DIR="${TARGET_DIR}/env"
SYSTEMD_DIR="/etc/systemd/system"
SERVICE_USER="${PROJECT_NAME}"
SERVICE_GROUP="${PROJECT_NAME}"

# Color output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Logging functions
log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

log_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Execution wrapper for dry-run mode
execute() {
    if [ "$DRY_RUN" = true ]; then
        echo -e "${YELLOW}[DRY-RUN]${NC} $*"
    else
        "$@"
    fi
}

# Display usage information
usage() {
    cat << EOF
Usage: $0 [options]

Options:
    install     Install the application
    uninstall   Uninstall the application
    -d, --dry-run   Dry-run mode (no actual changes)
    -h, --help      Display this help message

Examples:
    $0 install              # Install
    $0 install --dry-run    # Dry-run for installation
    $0 uninstall            # Uninstall
    $0 uninstall -d         # Dry-run for uninstallation

EOF
    exit 0
}

# Check for root privileges
check_root() {
    if [ "$EUID" -ne 0 ] && [ "$DRY_RUN" = false ]; then
        log_error "This script must be run as root"
        exit 1
    fi
}

# Check if already installed
check_installed() {
    if [ -d "$TARGET_DIR" ] && [ -f "${TARGET_DIR}/installer.sh" ]; then
        return 0  # Installed
    else
        return 1  # Not installed
    fi
}

# Detect if running from installed location
is_running_from_target() {
    local current_script="${BASH_SOURCE[0]}"
    local resolved_script="$(readlink -f "$current_script")"
    local target_installer="$(readlink -f "${TARGET_DIR}/installer.sh" 2>/dev/null || echo "")"
    
    if [ "$resolved_script" = "$target_installer" ] && [ -n "$target_installer" ]; then
        return 0  # Running from target directory
    else
        return 1  # Running from source directory
    fi
}

# Create service user and group
create_user() {
    log_info "Creating service user and group..."
    
    if ! getent group "$SERVICE_GROUP" > /dev/null 2>&1; then
        execute groupadd --system "$SERVICE_GROUP"
        log_success "Created group '$SERVICE_GROUP'"
    else
        log_info "Group '$SERVICE_GROUP' already exists"
    fi
    
    if ! getent passwd "$SERVICE_USER" > /dev/null 2>&1; then
        execute useradd --system --gid "$SERVICE_GROUP" \
            --home-dir "$TARGET_DIR" \
            --no-create-home \
            --shell /usr/sbin/nologin \
            "$SERVICE_USER"
        log_success "Created user '$SERVICE_USER'"
    else
        log_info "User '$SERVICE_USER' already exists"
    fi
}

# Create directories
create_directories() {
    log_info "Creating directories..."
    
    execute mkdir -p "$BIN_DIR"
    execute mkdir -p "$ETC_DIR"
    execute mkdir -p "$ENV_DIR"
    
    log_success "Created directories: $TARGET_DIR"
}

# Copy binary files
copy_binaries() {
    log_info "Copying binary files..."
    
    local bin_count=0
    if [ -d "${SCRIPT_DIR}/bin" ]; then
        for binary in "${SCRIPT_DIR}/bin"/*; do
            if [ -f "$binary" ]; then
                local bin_name=$(basename "$binary")
                execute cp "$binary" "${BIN_DIR}/${bin_name}"
                execute chmod 755 "${BIN_DIR}/${bin_name}"
                log_success "Copied: ${bin_name}"
                bin_count=$((bin_count + 1))
            fi
        done
    else
        log_warning "bin/ directory not found"
    fi
    
    log_info "Copied ${bin_count} binary file(s)"
}

# Copy configuration file
copy_config() {
    log_info "Copying configuration file..."
    
    if [ -f "${SCRIPT_DIR}/config.example.toml" ]; then
        local config_file="${ETC_DIR}/config.toml"
        
        if [ -f "$config_file" ] && [ "$DRY_RUN" = false ]; then
            log_warning "Existing configuration file found: $config_file"
            log_info "Creating backup: ${config_file}.bak"
            execute cp "$config_file" "${config_file}.bak"
        fi
        
        execute cp "${SCRIPT_DIR}/config.example.toml" "$config_file"
        execute chmod 644 "$config_file"
        log_success "Copied configuration file"
    else
        log_warning "Configuration file config.example.toml not found"
    fi
}

# Copy environment variable file
copy_env_file() {
    log_info "Copying environment variable file..."
    
    if [ -f "${SCRIPT_DIR}/${ENV_FILE}" ]; then
        local env_file="${ENV_DIR}/${ENV_FILE}"
        
        if [ -f "$env_file" ] && [ "$DRY_RUN" = false ]; then
            log_warning "Existing environment file found: $env_file"
            log_info "Creating backup: ${env_file}.bak"
            execute cp "$env_file" "${env_file}.bak"
        fi
        
        execute cp "${SCRIPT_DIR}/${ENV_FILE}" "$env_file"
        # Set restrictive permissions (readable only by owner)
        execute chmod 600 "$env_file"
        log_success "Copied environment file with secure permissions (600)"
    else
        log_warning "Environment file ${ENV_FILE} not found"
    fi
}

# Copy installer script itself
copy_installer() {
    log_info "Copying installer script..."
    
    local installer_dest="${TARGET_DIR}/installer.sh"
    
    execute cp "${SCRIPT_DIR}/$(basename "${BASH_SOURCE[0]}")" "$installer_dest"
    execute chmod 755 "$installer_dest"
    log_success "Copied installer script to ${installer_dest}"
    log_info "You can use this script for uninstallation: sudo ${installer_dest} uninstall"
}

# Create systemd unit files
create_systemd_units() {
    log_info "Creating systemd unit files..."
    
    if [ ! -d "${SCRIPT_DIR}/bin" ]; then
        log_warning "bin/ directory not found"
        return
    fi
    
    for binary in "${SCRIPT_DIR}/bin"/*; do
        if [ -f "$binary" ]; then
            local bin_name=$(basename "$binary")
            local service_name="${PROJECT_NAME}-${bin_name}.service"
            local service_file="${SYSTEMD_DIR}/${service_name}"
            
            log_info "Creating: ${service_name}"
            
            if [ "$DRY_RUN" = false ]; then
                cat > "$service_file" << EOF
[Unit]
Description=${PROJECT_NAME} ${bin_name} Service
After=network.target

[Service]
Type=simple
User=${SERVICE_USER}
Group=${SERVICE_GROUP}
WorkingDirectory=${TARGET_DIR}
EnvironmentFile=${ENV_DIR}/${ENV_FILE}
ExecStart=${BIN_DIR}/${bin_name}
Restart=on-failure
RestartSec=5s

# Security settings
PrivateTmp=true
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=${TARGET_DIR}

[Install]
WantedBy=multi-user.target
EOF
                chmod 644 "$service_file"
                log_success "Created: ${service_name}"
            else
                echo -e "${YELLOW}[DRY-RUN]${NC} cat > $service_file << EOF"
                echo "[Unit]"
                echo "Description=${PROJECT_NAME} ${bin_name} Service"
                echo "EnvironmentFile=${ENV_DIR}/${ENV_FILE}"
                echo "..."
            fi
        fi
    done
}

# Set file ownership and permissions
set_permissions() {
    log_info "Setting file ownership..."
    
    execute chown -R "${SERVICE_USER}:${SERVICE_GROUP}" "$TARGET_DIR"
    log_success "Set ownership"
}

# Reload systemd daemon
reload_systemd() {
    log_info "Reloading systemd daemon..."
    execute systemctl daemon-reload
    log_success "Reloaded systemd daemon"
}

# Enable services
enable_services() {
    log_info "Enabling services..."
    
    if [ ! -d "${SCRIPT_DIR}/bin" ]; then
        return
    fi
    
    for binary in "${SCRIPT_DIR}/bin"/*; do
        if [ -f "$binary" ]; then
            local bin_name=$(basename "$binary")
            local service_name="${PROJECT_NAME}-${bin_name}.service"
            
            execute systemctl enable "$service_name"
            log_success "Enabled: ${service_name}"
        fi
    done
    
    log_info "To start services: systemctl start ${PROJECT_NAME}-<service-name>"
}

# Install process
install() {
    log_info "=========================================="
    log_info "Starting installation"
    log_info "=========================================="
    
    if [ "$DRY_RUN" = true ]; then
        log_warning "Dry-run mode: No actual changes will be made"
    fi
    
    # Check if running from installed location
    if is_running_from_target; then
        log_error "Cannot install from the target directory"
        log_error "Please run the installer from the source directory"
        exit 1
    fi
    
    # Check if already installed
    if check_installed; then
        if [ "$DRY_RUN" = true ]; then
            log_warning "Installation directory already exists: $TARGET_DIR"
            log_info "Would prompt for reinstallation confirmation"
        else
            log_warning "Installation directory already exists: $TARGET_DIR"
            read -p "Do you want to reinstall? This will overwrite existing files (y/N): " -n 1 -r
            echo
            if [[ ! $REPLY =~ ^[Yy]$ ]]; then
                log_info "Installation cancelled"
                exit 0
            fi
            log_info "Proceeding with reinstallation..."
        fi
    fi
    
    check_root
    create_user
    create_directories
    copy_binaries
    copy_config
    copy_env_file
    copy_installer
    set_permissions
    create_systemd_units
    reload_systemd
    enable_services
    
    log_info "=========================================="
    log_success "Installation completed!"
    log_info "=========================================="
    log_info "Project name: ${PROJECT_NAME}"
    log_info "Installation directory: $TARGET_DIR"
    log_info "Environment file: ${ENV_DIR}/${ENV_FILE} (permissions: 600)"
    log_info "Installer script: ${TARGET_DIR}/installer.sh"
    log_info ""
    log_info "Start services: systemctl start ${PROJECT_NAME}-<service-name>"
    log_info "Check status: systemctl status ${PROJECT_NAME}-<service-name>"
    log_info "Uninstall: sudo ${TARGET_DIR}/installer.sh uninstall"
}

# Stop and disable services
stop_and_disable_services() {
    log_info "Stopping and disabling services..."
    
    # Search from bin directory
    if [ -d "$BIN_DIR" ]; then
        for binary in "${BIN_DIR}"/*; do
            if [ -f "$binary" ]; then
                local bin_name=$(basename "$binary")
                local service_name="${PROJECT_NAME}-${bin_name}.service"
                local service_file="${SYSTEMD_DIR}/${service_name}"
                
                if [ -f "$service_file" ]; then
                    execute systemctl stop "$service_name" 2>/dev/null || true
                    execute systemctl disable "$service_name" 2>/dev/null || true
                    log_success "Stopped and disabled: ${service_name}"
                fi
            fi
        done
    else
        log_warning "Binary directory not found: $BIN_DIR"
    fi
}

# Remove systemd unit files
remove_systemd_units() {
    log_info "Removing systemd unit files..."
    
    # Search from bin directory
    if [ -d "$BIN_DIR" ]; then
        for binary in "${BIN_DIR}"/*; do
            if [ -f "$binary" ]; then
                local bin_name=$(basename "$binary")
                local service_file="${SYSTEMD_DIR}/${PROJECT_NAME}-${bin_name}.service"
                
                if [ -f "$service_file" ]; then
                    execute rm -f "$service_file"
                    log_success "Removed: ${PROJECT_NAME}-${bin_name}.service"
                fi
            fi
        done
    else
        log_warning "Binary directory not found: $BIN_DIR"
    fi
}

# Remove directories
remove_directories() {
    log_info "Removing installation directory..."
    
    if [ -d "$TARGET_DIR" ]; then
        execute rm -rf "$TARGET_DIR"
        log_success "Removed: $TARGET_DIR"
    else
        log_info "Directory does not exist: $TARGET_DIR"
    fi
}

# Remove service user and group
remove_user() {
    log_info "Removing service user and group..."
    
    if getent passwd "$SERVICE_USER" > /dev/null 2>&1; then
        execute userdel "$SERVICE_USER"
        log_success "Removed user '$SERVICE_USER'"
    fi
    
    if getent group "$SERVICE_GROUP" > /dev/null 2>&1; then
        execute groupdel "$SERVICE_GROUP"
        log_success "Removed group '$SERVICE_GROUP'"
    fi
}

# Uninstall process
uninstall() {
    log_info "=========================================="
    log_info "Starting uninstallation"
    log_info "=========================================="
    
    if [ "$DRY_RUN" = true ]; then
        log_warning "Dry-run mode: No actual changes will be made"
    fi
    
    # Check if installed
    if ! check_installed; then
        log_error "Installation not found at: $TARGET_DIR"
        log_error "Nothing to uninstall"
        exit 1
    fi
    
    # Confirm uninstallation if not in dry-run mode
    if [ "$DRY_RUN" = false ]; then
        log_warning "This will completely remove all installed files and services"
        read -p "Are you sure you want to uninstall? (y/N): " -n 1 -r
        echo
        if [[ ! $REPLY =~ ^[Yy]$ ]]; then
            log_info "Uninstallation cancelled"
            exit 0
        fi
    else
        log_info "Would prompt for uninstallation confirmation"
    fi
    
    check_root
    stop_and_disable_services
    remove_systemd_units
    reload_systemd
    remove_directories
    remove_user
    
    log_info "=========================================="
    log_success "Uninstallation completed"
    log_info "=========================================="
}

# Main function
main() {
    local action=""
    DRY_RUN=false
    
    # No arguments provided
    if [ $# -eq 0 ]; then
        usage
    fi
    
    # Parse arguments
    while [ $# -gt 0 ]; do
        case "$1" in
            install)
                action="install"
                shift
                ;;
            uninstall)
                action="uninstall"
                shift
                ;;
            -d|--dry-run)
                DRY_RUN=true
                shift
                ;;
            -h|--help)
                usage
                ;;
            *)
                log_error "Unknown option: $1"
                usage
                ;;
        esac
    done
    
    # Execute action
    case "$action" in
        install)
            install
            ;;
        uninstall)
            uninstall
            ;;
        *)
            log_error "Please specify an action: install or uninstall"
            usage
            ;;
    esac
}

# Execute script
main "$@"
