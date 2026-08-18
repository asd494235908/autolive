"""Windows 后台 Worker 的子进程窗口策略。"""

from __future__ import annotations

import os
import subprocess


def subprocess_creation_flags() -> int:
    return int(getattr(subprocess, "CREATE_NO_WINDOW", 0))


def install_hidden_subprocess_policy() -> None:
    """让当前 Worker 及第三方依赖创建的控制台子进程保持后台运行。"""
    if os.name != "nt" or getattr(subprocess.Popen, "_autolive_no_window", False):
        return

    original_popen = subprocess.Popen

    class HiddenBackgroundPopen(original_popen):
        _autolive_no_window = True

        def __init__(self, *args, **kwargs) -> None:
            existing_flags = int(kwargs.get("creationflags", 0) or 0)
            kwargs["creationflags"] = existing_flags | subprocess_creation_flags()
            super().__init__(*args, **kwargs)

    subprocess.Popen = HiddenBackgroundPopen
