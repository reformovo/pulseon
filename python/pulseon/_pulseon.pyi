import os

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

class Client:
    def create_project(
        self, name: str, project_id: str | None = None
    ) -> Project: ...
    def create_run(
        self, project_id: str, name: str, run_id: str | None = None
    ) -> Run: ...

def init(path: str | os.PathLike[str]) -> Client: ...
