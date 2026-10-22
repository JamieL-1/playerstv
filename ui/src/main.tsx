import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { createHashRouter, RouterProvider } from "react-router";
import { AppShell } from "@/components/shell/AppShell";
import { CloseDialog } from "@/components/shell/CloseDialog";
import { Toaster } from "@/components/ui/sonner";
import { TooltipProvider } from "@/components/ui/tooltip";
import { LibraryPage } from "@/pages/LibraryPage";
import { WatchPage } from "@/pages/WatchPage";
import { SettingsLayout } from "@/pages/settings/SettingsLayout";
import { ThemeProvider, useTheme } from "@/theme/ThemeProvider";
import "./index.css";

const queryClient = new QueryClient({ defaultOptions: { queries: { retry: 1, refetchOnWindowFocus: false } } });

// UI tests (debug builds) can start on a given page; see crates/ui/src/window.rs.
const testRoute = (window as { __PTV_TEST_ROUTE__?: string }).__PTV_TEST_ROUTE__;
if (testRoute && !location.hash) location.hash = `#${testRoute}`;

// Hash routing: the app is served from tauri:// with no server-side routes.
const router = createHashRouter([
  {
    path: "/",
    element: <AppShell />,
    children: [
      { index: true, element: <LibraryPage /> },
      { path: "game/:gameId", element: <LibraryPage /> },
    ],
  },
  { path: "/settings/:section?", element: <SettingsLayout /> },
  { path: "/settings/games/:gameId", element: <SettingsLayout /> },
  { path: "/watch", element: <WatchPage /> },
]);

function ThemedToaster() {
  const { mode } = useTheme();
  return <Toaster theme={mode} position="bottom-right" />;
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <ThemeProvider>
        <TooltipProvider delayDuration={200}>
          <RouterProvider router={router} />
          <ThemedToaster />
          <CloseDialog />
        </TooltipProvider>
      </ThemeProvider>
    </QueryClientProvider>
  </StrictMode>,
);
