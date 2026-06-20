import {
  Box,
  Card,
  CardContent,
  Grid,
  Typography,
} from "@mui/material";
import InventoryIcon from "@mui/icons-material/Inventory";
import AltRouteIcon from "@mui/icons-material/AltRoute";
import ArticleIcon from "@mui/icons-material/Article";
import TuneIcon from "@mui/icons-material/Tune";

const stats = [
  { label: "Packages", value: "—", icon: <InventoryIcon />, color: "#90caf9" },
  { label: "Iflows", value: "—", icon: <AltRouteIcon />, color: "#a5d6a7" },
  { label: "Configurations", value: "—", icon: <ArticleIcon />, color: "#fff59d" },
  { label: "Service Endpoints", value: "—", icon: <TuneIcon />, color: "#ce93d8" },
];

export default function Dashboard() {
  return (
    <Box>
      <Typography variant="h4" gutterBottom>Dashboard</Typography>
      <Typography variant="body1" color="text.secondary" sx={{ mb: 3 }}>
        Select a profile and connect to view your tenant data.
      </Typography>

      <Grid container spacing={3}>
        {stats.map((s) => (
          <Grid size={{ xs: 12, sm: 6, md: 3 }} key={s.label}>
            <Card>
              <CardContent>
                <Box sx={{ display: "flex", alignItems: "center", gap: 2 }}>
                  <Box sx={{ color: s.color }}>{s.icon}</Box>
                  <Box>
                    <Typography variant="h4">{s.value}</Typography>
                    <Typography variant="body2" color="text.secondary">{s.label}</Typography>
                  </Box>
                </Box>
              </CardContent>
            </Card>
          </Grid>
        ))}
      </Grid>
    </Box>
  );
}