#include "daemon.h"
#include <iostream>
#include <csignal>

static slate::SlatedDaemon* g_daemon = nullptr;

static void signal_handler(int /*sig*/) {
    if (g_daemon) g_daemon->request_shutdown();
}

int main(int /*argc*/, char* /*argv*/[]) {
    slate::SlatedDaemon daemon;
    g_daemon = &daemon;

    std::signal(SIGTERM, signal_handler);
    std::signal(SIGINT, signal_handler);

    try {
        daemon.start();
        daemon.run();
    } catch (const std::exception& e) {
        std::cerr << "slated: " << e.what() << std::endl;
        return 1;
    }

    daemon.shutdown();
    return 0;
}
