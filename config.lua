return {
    services = {
        backend = {
            command = "python",
            args = {"-m", "http.server", "8080"},
            health_check = {
                probe = {
                    Http = {
                        port = 8080,
                        path = "/"
                    }
                }
            }
        },
        worker = {
            command = "python",
            args = {"-c", "import time, sys; [print(f'Working {i}', flush=True) or time.sleep(1) for i in range(100)]"},
            -- This health check is intentionally incorrect to validate that the health check fails properly
            health_check = {
                probe = {
                    Http = {
                        port = 8081,
                        path = "/"
                    }
                }
            }
        },
        cwd = {
            command = "python",
            args = {"-c", "import os; print(os.getcwd())"}
        },
        env = {
            command = "python",
            args = {"-c", "import os; print(os.environ['test'])"},
            env = {
                test = 'hello there!'
            }
        }
    }
}