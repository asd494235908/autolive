using System.Windows;
using System.Windows.Controls;

namespace GpAutoLive.App;

public partial class MainWindow
{
    // Only presentation properties: no media, configuration, authentication or event changes.
    private void UpdateResponsiveLayout(Size size)
    {
        if (WorkbenchSurface is null || MediaWorkspace is null)
        {
            return;
        }

        var width = size.Width / ResponsiveShellScaleTransform.ScaleX;
        var height = (size.Height - 45) / ResponsiveShellScaleTransform.ScaleY;
        ShellContent.Height = Math.Max(600, height);
        var columns = width >= 1250 ? 3 : width >= 1000 ? 2 : 1;
        WorkbenchSurface.ColumnDefinitions[0].Width = new GridLength(columns == 3 ? 24 : columns == 2 ? 32 : 1, GridUnitType.Star);
        WorkbenchSurface.ColumnDefinitions[1].Width = new GridLength(columns > 1 ? 10 : 0);
        WorkbenchSurface.ColumnDefinitions[2].Width = columns > 1 ? new GridLength(columns == 3 ? 48 : 68, GridUnitType.Star) : new GridLength(0);
        WorkbenchSurface.ColumnDefinitions[3].Width = new GridLength(columns == 3 ? 10 : 0);
        WorkbenchSurface.ColumnDefinitions[4].Width = columns == 3 ? new GridLength(28, GridUnitType.Star) : new GridLength(0);
        Grid.SetColumn(ParameterWorkspace, columns > 1 ? 2 : 0);
        Grid.SetRow(ParameterWorkspace, columns > 1 ? 0 : 1);
        Grid.SetColumn(OutputWorkspace, columns == 3 ? 4 : 0);
        Grid.SetRow(OutputWorkspace, columns == 3 ? 0 : columns == 2 ? 1 : 2);
        Grid.SetColumnSpan(OutputWorkspace, columns == 2 ? 5 : 1);
        UpdateWorkbenchHeight(WorkbenchViewport.ActualHeight);
        OutputWorkspace.Margin = new Thickness(0, columns == 3 ? 0 : 10, 0, 0);
        ParameterWorkspace.Margin = new Thickness(0, columns == 1 ? 10 : 0, 0, 0);
    }

    private void WorkbenchViewport_SizeChanged(object sender, SizeChangedEventArgs e) =>
        UpdateWorkbenchHeight(e.NewSize.Height);

    private void UpdateWorkbenchHeight(double viewportHeight)
    {
        if (MediaWorkspace is null || ParameterWorkspace is null || OutputWorkspace is null)
        {
            return;
        }

        var sideBySide = Grid.GetRow(ParameterWorkspace) == 0;
        var height = Math.Max(420, viewportHeight - 19);
        MediaWorkspace.Height = sideBySide ? height : 450;
        ParameterWorkspace.Height = sideBySide ? height : 820;
        OutputWorkspace.Height = Grid.GetRow(OutputWorkspace) == 0 ? height : double.NaN;
    }
}
