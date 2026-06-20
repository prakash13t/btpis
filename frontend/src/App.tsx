import { Routes, Route, Navigate } from "react-router-dom";
import { ThemeProvider, createTheme, CssBaseline } from "@mui/material";
import Layout from "./Layout";
import Dashboard from "./pages/Dashboard";

const darkTheme = createTheme({
  palette: { mode: "dark", primary: { main: "#90caf9" } },
});

function App() {
  return (
    <ThemeProvider theme={darkTheme}>
      <CssBaseline />
      <Layout>
        <Routes>
          <Route path="/" element={<Dashboard />} />
          <Route path="*" element={<Navigate to="/" replace />} />
        </Routes>
      </Layout>
    </ThemeProvider>
  );
}

export default App;