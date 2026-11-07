public static class Health
{
    public static int points = 3;

    public static void ApplyDamage(int amount)
    {
        points -= amount;
    }
}
