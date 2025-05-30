#!/usr/bin/env bash
export XDG_SESSION_DESKTOP=niri
export XDG_SESSION_TYPE=wayland

export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
export WAYLAND_DISPLAY=${WAYLAND_DISPLAY:-wayland-0}

# Recently, arch linux stopped starting the user's systemd on boot, so this starts it manually if needed
if [[ ! -e ${XDG_RUNTIME_DIR}/bus ]]; then
  # https://github.com/microsoft/WSL/issues/8842
  # Restart systemd for user

  # Starting recently, arch linux takes a few seconds for systemd to start, it used to be instant
  while ! sudo systemctl restart user@$(id -u); do
    :
  done

  while [[ ! -e ${XDG_RUNTIME_DIR}/bus ]]; do
    printf "Waiting for user's systemd to start..."
    sleep 1
  done
fi

# Remove WSLG's xwayland so Sway can start it's own
sudo -s <<EOF
  umount /tmp/.X11-unix
  rm -rf /tmp/.X11-unix
  chmod 700 $XDG_RUNTIME_DIR
EOF

mkdir /tmp/.X11-unix
chmod 01777 /tmp/.X11-unix

# Fix incase this soft link is lost
ln -sf /mnt/wslg/runtime-dir/wayland-0 ${XDG_RUNTIME_DIR}/${WAYLAND_DISPLAY}
ln -sf /mnt/wslg/runtime-dir/wayland-0.lock ${XDG_RUNTIME_DIR}/${WAYLAND_DISPLAY}.lock

niri
