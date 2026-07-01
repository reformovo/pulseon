import os
from typing import overload

class Diagnostics:
    accepted_reports: int
    dropped_reports: int
    failed_reports: int

class Project:
    project_id: str
    name: str
    created_at: str

class Run:
    run_id: str
    project_id: str
    name: str
    status: str
    created_at: str
    started_at: str
    finished_at: str | None
    @overload
    def log(self, key: str, value: float, /) -> None: ...
    @overload
    def log(self, key: str, step: int, value: float, /) -> None: ...

class Client:
    def create_project(
        self, name: str, project_id: str | None = None
    ) -> Project: ...
    def create_run(
        self, project_id: str, name: str, run_id: str | None = None
    ) -> Run: ...
    def diagnostics(self) -> Diagnostics: ...

def init(path: str | os.PathLike[str]) -> Client: ...
