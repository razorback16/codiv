#include "app.h"
#include <iostream>
#include <csignal>

static slate::SlateApp* g_app = nullptr;

static void signal_handler(int sig) {
    if (sig == SIGINT && g_app) {
        g_app->forward_signal(sig);
    }
    if (sig == SIGTERM && g_app) {
        g_app->request_shutdown();
    }
}

int main(int argc, char* argv[]) {
    (void)argc;
    (void)argv;

    slate::SlateApp app;
    g_app = &app;

    std::signal(SIGINT, signal_handler);
    std::signal(SIGTERM, signal_handler);

    try {
        app.init();
        return app.run();
    } catch (const std::exception& e) {
        std::cerr << "slate: " << e.what() << std::endl;
        return 1;
    }
}
